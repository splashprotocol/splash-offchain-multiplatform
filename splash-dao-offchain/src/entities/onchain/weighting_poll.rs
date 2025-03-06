use cml_chain::address::EnterpriseAddress;
use cml_chain::assets::AssetName;
use cml_chain::certs::StakeCredential;
use cml_chain::plutus::{ConstrPlutusData, PlutusData, PlutusV2Script};
use cml_chain::transaction::{DatumOption, TransactionOutput};
use cml_chain::utils::BigInteger;
use cml_chain::{OrderedHashMap, PolicyId, Value};
use cml_crypto::RawBytesEncoding;
use derive_more::From;
use log::trace;
use serde::{Deserialize, Serialize};
use spectrum_cardano_lib::transaction::TransactionOutputExtension;
use spectrum_cardano_lib::types::TryFromPData;
use spectrum_offchain_cardano::deployment::{test_address, DeployedScriptInfo};
use uplc_pallas_codec::utils::Int;

use spectrum_cardano_lib::plutus_data::{
    ConstrPlutusDataExtension, DatumExtension, IntoPlutusData, PlutusDataExtension,
};
use spectrum_cardano_lib::{NetworkId, TaggedAmount};
use spectrum_offchain::domain::{Has, Stable};
use spectrum_offchain::ledger::{IntoLedger, TryFromLedger};
use spectrum_offchain_cardano::parametrized_validators::apply_params_validator_plutus_v2;
use uplc_pallas_primitives::{BoundedBytes, MaybeIndefArray};

use crate::assets::Splash;
use crate::constants::time::{
    COOLDOWN_PERIOD_EXTRA_BUFFER, COOLDOWN_PERIOD_MILLIS, DISTRIBUTE_INFLATION_START_DELAY_MILLIS,
};
use crate::constants::SPLASH_NAME;
use crate::deployment::{DaoScriptData, ProtocolValidator};
use crate::entities::onchain::smart_farm::FarmId;
use crate::entities::Snapshot;
use crate::protocol_config::{GTAuthPolicy, MintWPAuthPolicy, SplashPolicy, WeightingPowerPolicy};
use crate::routines::inflation::actions::compute_epoch_asset_name;
use crate::routines::inflation::{slot_to_epoch, TimedOutputRef};
use crate::time::{epoch_end, epoch_start, NetworkTime, ProtocolEpoch};
use crate::GenesisEpochStartTime;

pub type WeightingPollSnapshot = Snapshot<WeightingPoll, TimedOutputRef>;

#[derive(
    Copy,
    Clone,
    PartialEq,
    Eq,
    Ord,
    PartialOrd,
    From,
    Serialize,
    Deserialize,
    Hash,
    Debug,
    derive_more::Display,
)]
pub struct WeightingPollId(pub ProtocolEpoch);

#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct WeightingPoll {
    pub epoch: ProtocolEpoch,
    pub distribution: Vec<(FarmId, u64)>,
    pub emission_rate: TaggedAmount<Splash>,
    /// Note: weighting power is not determined until vote stage.
    pub weighting_power: Option<u64>,
    pub eliminated: bool,
}

impl Stable for WeightingPoll {
    type StableId = WeightingPollId;
    fn stable_id(&self) -> Self::StableId {
        WeightingPollId(self.epoch)
    }
    fn is_quasi_permanent(&self) -> bool {
        true
    }
}

impl<Ctx> IntoLedger<TransactionOutput, Ctx> for WeightingPoll
where
    Ctx: Has<SplashPolicy>
        + Has<GenesisEpochStartTime>
        + Has<MintWPAuthPolicy>
        + Has<WeightingPowerPolicy>
        + Has<GTAuthPolicy>
        + Has<NetworkId>,
{
    fn into_ledger(self, ctx: Ctx) -> TransactionOutput {
        let wp_auth_policy = ctx.select::<MintWPAuthPolicy>().0;
        let datum = create_datum(
            &self,
            ctx.select::<GenesisEpochStartTime>(),
            ctx.select::<WeightingPowerPolicy>().0,
        );

        let cred = StakeCredential::new_script(wp_auth_policy);
        let address = EnterpriseAddress::new(ctx.select::<NetworkId>().into(), cred).to_address();

        // Note: We don't add wp_auth_token to this output here.
        let mut bundle = OrderedHashMap::new();
        bundle.insert(
            ctx.select::<SplashPolicy>().0,
            OrderedHashMap::from_iter(vec![(
                AssetName::try_from(SPLASH_NAME.as_bytes().to_vec()).unwrap(),
                self.emission_rate.untag(),
            )]),
        );
        let amount = Value::new(1680900, bundle.into());
        TransactionOutput::new(address, amount, Some(DatumOption::new_datum(datum)), None)
    }
}

fn create_datum(
    wpoll: &WeightingPoll,
    genesis_epoch_start_time: GenesisEpochStartTime,
    weighting_power_policy: PolicyId,
) -> PlutusData {
    let distribution_pd = distribution_to_plutus_data(&wpoll.distribution);
    let deadline_inner = wpoll.voting_deadline_time(genesis_epoch_start_time);
    let deadline = PlutusData::new_integer(BigInteger::from(deadline_inner));
    let emission_rate = PlutusData::new_integer(BigInteger::from(wpoll.emission_rate.untag()));
    let weighting_power_policy_pd = PlutusData::new_bytes(weighting_power_policy.to_raw_bytes().to_vec());

    PlutusData::ConstrPlutusData(ConstrPlutusData::new(
        0,
        vec![
            distribution_pd,
            deadline,
            emission_rate,
            weighting_power_policy_pd,
        ],
    ))
}

pub struct WeightingOngoing;
pub struct DistributionOngoing(FarmId, u64);
impl DistributionOngoing {
    pub fn farm_id(&self) -> FarmId {
        self.0
    }
    pub fn farm_weight(&self) -> u64 {
        self.1
    }
}

pub enum PollState {
    WeightingOngoing(WeightingOngoing),
    DistributionOngoing(DistributionOngoing),
    WaitingForDistributionToStart,
    PollExhaustedAndReadyToEliminate,
    PollExhaustedButNotReadyToEliminate,
    Eliminated,
}

impl WeightingPoll {
    pub fn can_be_eliminated(&self, genesis: GenesisEpochStartTime, time_now: NetworkTime) -> bool {
        let epoch_end = epoch_end(genesis, self.epoch);
        let past_cooling_off_period =
            time_now > epoch_end + COOLDOWN_PERIOD_MILLIS + COOLDOWN_PERIOD_EXTRA_BUFFER;
        self.distribution_finished()
            && self.weighting_power.is_some()
            && !self.eliminated
            && past_cooling_off_period
    }

    pub fn reserves_splash(&self) -> u64 {
        self.distribution.iter().fold(0, |acc, (_, i)| acc + *i)
    }

    pub fn next_farm(&self) -> Option<(FarmId, u64)> {
        self.distribution.iter().find(|x| x.1 > 0).copied()
    }

    pub fn state(&self, genesis: GenesisEpochStartTime, time_now: NetworkTime) -> PollState {
        if self.weighting_open(genesis, time_now) {
            PollState::WeightingOngoing(WeightingOngoing)
        } else {
            match self.next_farm() {
                None => {
                    if self.eliminated {
                        PollState::Eliminated
                    } else if self.can_be_eliminated(genesis, time_now) {
                        PollState::PollExhaustedAndReadyToEliminate
                    } else {
                        PollState::PollExhaustedButNotReadyToEliminate
                    }
                }
                Some((farm, weight)) => {
                    let epoch_end = epoch_end(genesis, self.epoch);
                    let can_start_distribution =
                        time_now > epoch_end + DISTRIBUTE_INFLATION_START_DELAY_MILLIS;
                    if can_start_distribution {
                        PollState::DistributionOngoing(DistributionOngoing(farm, weight))
                    } else {
                        PollState::WaitingForDistributionToStart
                    }
                }
            }
        }
    }

    pub fn voting_deadline_time(&self, genesis: GenesisEpochStartTime) -> NetworkTime {
        epoch_end(genesis, self.epoch)
    }

    pub fn apply_votes(&mut self, order_distribution: &[(FarmId, u64)]) {
        // assert_eq!(self.distribution.len(), order_distribution.len());
        for (farm_id, weight) in order_distribution {
            let ix = self
                .distribution
                .iter()
                .position(|&(f_id, _w)| f_id == *farm_id)
                .unwrap();
            self.distribution[ix].1 += *weight;
        }
    }

    fn weighting_open(&self, genesis: GenesisEpochStartTime, time_now: NetworkTime) -> bool {
        let e_start = epoch_start(genesis, self.epoch);
        let e_end = epoch_end(genesis, self.epoch);
        e_start < time_now && e_end > time_now
    }

    fn distribution_finished(&self) -> bool {
        self.reserves_splash() == 0
    }
}

impl<C> TryFromLedger<TransactionOutput, C> for WeightingPollSnapshot
where
    C: Has<GenesisEpochStartTime>
        + Has<DeployedScriptInfo<{ ProtocolValidator::MintWpAuthPolicy as u8 }>>
        + Has<MintWPAuthPolicy>
        + Has<NetworkId>
        + Has<TimedOutputRef>,
{
    fn try_from_ledger(repr: &TransactionOutput, ctx: &C) -> Option<Self> {
        if test_address(repr.address(), ctx) {
            let value = repr.value().clone();
            let WeightingPollConfig {
                distribution,
                emission_rate,
                weighting_power_policy,
                ..
            } = WeightingPollConfig::try_from_pd(repr.datum()?.into_pd()?)?;

            let TimedOutputRef { output_ref, slot } = ctx.select::<TimedOutputRef>();
            let genesis_time_millis = ctx.select::<GenesisEpochStartTime>();
            let network_id = ctx.select::<NetworkId>();
            let current_epoch = slot_to_epoch(slot.0, genesis_time_millis, network_id);
            let distribution = distribution.into_iter().map(|f| (f.id, f.weight)).collect();
            let wp_auth_policy = ctx.select::<MintWPAuthPolicy>().0;

            for epoch in 0..=current_epoch.0 {
                // wp_auth_token and weighting_power tokens have same asset name
                let token_asset_name = compute_epoch_asset_name(epoch);
                if value.multiasset.get(&wp_auth_policy, &token_asset_name).is_some() {
                    let weighting_power = value.multiasset.get(&weighting_power_policy, &token_asset_name);

                    trace!(
                        "FOUND WEIGHTING_POLL: epoch: {}, weighting_power: {:?}",
                        epoch,
                        weighting_power
                    );
                    let weighting_poll = WeightingPoll {
                        epoch,
                        distribution,
                        emission_rate: TaggedAmount::new(emission_rate),
                        weighting_power,
                        eliminated: false,
                    };
                    return Some(Snapshot::new(weighting_poll, TimedOutputRef { output_ref, slot }));
                }
            }
        }
        None
    }
}

pub fn distribution_to_plutus_data(distribution: &[(FarmId, u64)]) -> PlutusData {
    let mut list = vec![];
    for (farm_id, weight) in distribution {
        let farm = Farm {
            id: *farm_id,
            weight: *weight,
        };
        list.push(farm.into_pd());
    }
    PlutusData::new_list(list)
}

pub struct Farm {
    id: FarmId,
    weight: u64,
}

impl IntoPlutusData for Farm {
    fn into_pd(self) -> PlutusData {
        PlutusData::ConstrPlutusData(ConstrPlutusData::new(
            0,
            vec![
                self.id.into_pd(),
                PlutusData::new_integer(BigInteger::from(self.weight)),
            ],
        ))
    }
}

impl TryFromPData for Farm {
    fn try_from_pd(data: PlutusData) -> Option<Self> {
        let mut cpd = data.into_constr_pd()?;
        if cpd.alternative == 0 {
            let id = FarmId::try_from_pd(cpd.take_field(0)?)?;
            if let PlutusData::Integer(w) = cpd.take_field(1)? {
                return Some(Farm {
                    id,
                    weight: w.as_u64()?,
                });
            }
        }
        None
    }
}

impl IntoPlutusData for WeightingPollConfig {
    fn into_pd(self) -> PlutusData {
        let list: Vec<_> = self.distribution.into_iter().map(|f| f.into_pd()).collect();
        let deadline = PlutusData::new_integer(BigInteger::from(self.deadline));
        let emission_rate = PlutusData::new_integer(BigInteger::from(self.emission_rate));
        let distribution = PlutusData::new_list(list);
        let weighting_power_policy =
            PlutusData::new_bytes(self.weighting_power_policy.to_raw_bytes().to_vec());
        let fields = vec![distribution, deadline, emission_rate, weighting_power_policy];
        PlutusData::ConstrPlutusData(ConstrPlutusData::new(0, fields))
    }
}

impl TryFromPData for WeightingPollConfig {
    fn try_from_pd(data: PlutusData) -> Option<Self> {
        let mut cpd = data.into_constr_pd()?;
        let distribution = cpd.take_field(0)?.into_vec_pd(Farm::try_from_pd)?;
        let deadline = cpd.take_field(1)?.into_u64()?;
        let emission_rate = cpd.take_field(2)?.into_u64()?;
        let weighting_power_policy = PolicyId::from_raw_bytes(&cpd.take_field(3)?.into_bytes()?).ok()?;

        Some(Self {
            distribution,
            deadline,
            emission_rate,
            weighting_power_policy,
        })
    }
}

pub struct WeightingPollConfig {
    /// Farms available for weighting.
    /// Assumption: Distribution consists of only valid active farms. Guaranteed by WP Factory.
    distribution: Vec<Farm>,
    /// Deadline of the poll.
    /// Assumption: Deadline corresponds to the end of the actual epoch this wp belongs to.
    ///             Guaranteed by WP Factory.
    deadline: u64,
    /// Emission rate in this epoch.
    /// Assumption: Current rate corresponds to the actual epoch this wp belongs to.
    ///             Guaranteed by WP Factory.
    emission_rate: u64,
    /// The validator will look for a token = (`w_power_policy`, `binder`) to validate usage of voting power in current epoch.
    weighting_power_policy: PolicyId,
}

pub fn unsafe_update_wp_state(data: &mut PlutusData, new_distribution: &[(FarmId, u64)]) {
    let cpd = data.get_constr_pd_mut().unwrap();
    cpd.set_field(0, distribution_to_plutus_data(new_distribution))
}

pub enum PollAction {
    /// Until epoch end.
    Vote,
    /// After epoch end.
    Distribute {
        /// Index of the farm.
        farm_ix: u32,
        /// Index of the farm input.
        farm_in_ix: u32,
    },
    Destroy,
}

impl IntoPlutusData for PollAction {
    fn into_pd(self) -> PlutusData {
        let inner = match self {
            PollAction::Vote => PlutusData::ConstrPlutusData(ConstrPlutusData::new(0, vec![])),
            PollAction::Distribute { farm_ix, farm_in_ix } => {
                PlutusData::ConstrPlutusData(ConstrPlutusData::new(
                    1,
                    vec![
                        PlutusData::Integer(BigInteger::from(farm_ix)),
                        PlutusData::Integer(BigInteger::from(farm_in_ix)),
                    ],
                ))
            }
            PollAction::Destroy => PlutusData::ConstrPlutusData(ConstrPlutusData::new(2, vec![])),
        };

        // Need this wrapping since `weighting_poll` is a multivalidator.
        PlutusData::new_constr_plutus_data(ConstrPlutusData::new(1, vec![inner]))
    }
}

pub enum MintAction {
    MintAuthToken {
        factory_in_ix: u32,
        inflation_box_in_ix: u32,
    },
    BurnAuthToken,
}

impl IntoPlutusData for MintAction {
    fn into_pd(self) -> PlutusData {
        match self {
            MintAction::MintAuthToken {
                factory_in_ix,
                inflation_box_in_ix,
            } => PlutusData::ConstrPlutusData(ConstrPlutusData::new(
                0,
                vec![
                    PlutusData::Integer(BigInteger::from(factory_in_ix)),
                    PlutusData::Integer(BigInteger::from(inflation_box_in_ix)),
                ],
            )),
            MintAction::BurnAuthToken => PlutusData::ConstrPlutusData(ConstrPlutusData::new(1, vec![])),
        }
    }
}

pub const MIN_ADA_IN_BOX: u64 = 1_000_000;

/// Note that the this is a multivalidator, and can serve as the script that guards the
/// weighting_poll.
pub fn compute_mint_wp_auth_token_validator(
    splash_policy: PolicyId,
    farm_auth_policy: PolicyId,
    factory_auth_policy: PolicyId,
    inflation_box_auth_policy: PolicyId,
    zeroth_epoch_start: u64,
) -> PlutusV2Script {
    let params_pd = uplc::PlutusData::Array(MaybeIndefArray::Indef(vec![
        uplc::PlutusData::BoundedBytes(BoundedBytes::from(splash_policy.to_raw_bytes().to_vec())),
        uplc::PlutusData::BoundedBytes(BoundedBytes::from(farm_auth_policy.to_raw_bytes().to_vec())),
        uplc::PlutusData::BoundedBytes(BoundedBytes::from(factory_auth_policy.to_raw_bytes().to_vec())),
        uplc::PlutusData::BoundedBytes(BoundedBytes::from(
            inflation_box_auth_policy.to_raw_bytes().to_vec(),
        )),
        uplc::PlutusData::BigInt(uplc::BigInt::Int(Int::from(zeroth_epoch_start as i64))),
    ]));
    apply_params_validator_plutus_v2(
        params_pd,
        &DaoScriptData::global().mint_wp_auth_token.script_bytes,
    )
}

#[cfg(test)]
mod tests {

    #[test]
    fn test_wp_zzz() {
        let bytes: Vec<u8> = vec![
            193, 193, 97, 136, 48, 101, 63, 181, 108, 59, 225, 140, 150, 101, 161, 42, 147, 115, 64, 230,
            176, 192, 255, 157, 231, 252, 218, 178,
        ];
        println!("{}", hex::encode(&bytes));
    }
}
