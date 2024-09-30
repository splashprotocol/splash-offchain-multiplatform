use cml_chain::address::EnterpriseAddress;
use cml_chain::assets::AssetName;
use cml_chain::certs::StakeCredential;
use cml_chain::plutus::{ConstrPlutusData, ExUnits, PlutusData, PlutusV2Script};
use cml_chain::transaction::{DatumOption, TransactionOutput};
use cml_chain::utils::BigInteger;
use cml_chain::{OrderedHashMap, PolicyId, Value};
use cml_crypto::RawBytesEncoding;
use derive_more::From;
use serde::{Deserialize, Serialize};
use spectrum_cardano_lib::transaction::TransactionOutputExtension;
use spectrum_cardano_lib::types::TryFromPData;
use spectrum_offchain_cardano::deployment::{test_address, DeployedScriptInfo};
use uplc_pallas_codec::utils::{Int, PlutusBytes};

use spectrum_cardano_lib::plutus_data::{
    ConstrPlutusDataExtension, DatumExtension, IntoPlutusData, PlutusDataExtension,
};
use spectrum_cardano_lib::{OutputRef, TaggedAmount, Token};
use spectrum_offchain::data::{Has, HasIdentifier, Identifier, Stable};
use spectrum_offchain::ledger::{IntoLedger, TryFromLedger};
use spectrum_offchain_cardano::parametrized_validators::apply_params_validator;

use crate::assets::Splash;
use crate::constants::{MINT_WP_AUTH_TOKEN_SCRIPT, SPLASH_NAME};
use crate::deployment::ProtocolValidator;
use crate::entities::onchain::smart_farm::FarmId;
use crate::entities::onchain::voting_escrow::compute_mint_weighting_power_policy_id;
use crate::entities::Snapshot;
use crate::protocol_config::{GTAuthPolicy, MintWPAuthPolicy, NodeMagic, SplashPolicy, WeightingPowerPolicy};
use crate::routines::inflation::actions::compute_epoch_asset_name;
use crate::time::{epoch_end, epoch_start, NetworkTime, ProtocolEpoch};
use crate::{CurrentEpoch, GenesisEpochStartTime};

pub type WeightingPollSnapshot = Snapshot<WeightingPoll, OutputRef>;

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

impl Identifier for WeightingPollId {
    type For = WeightingPollSnapshot;
}

#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct WeightingPoll {
    pub epoch: ProtocolEpoch,
    pub distribution: Vec<(FarmId, u64)>,
    pub emission_rate: TaggedAmount<Splash>,
    /// Note: weighting power is not determined until vote stage.
    pub weighting_power: Option<u64>,
}

impl HasIdentifier for WeightingPollSnapshot {
    type Id = WeightingPollId;

    fn identifier(&self) -> Self::Id {
        WeightingPollId(self.0.epoch)
    }
}

impl<Ctx> IntoLedger<TransactionOutput, Ctx> for WeightingPoll
where
    Ctx: Has<SplashPolicy>
        + Has<GenesisEpochStartTime>
        + Has<MintWPAuthPolicy>
        + Has<WeightingPowerPolicy>
        + Has<GTAuthPolicy>
        + Has<NodeMagic>,
{
    fn into_ledger(self, ctx: Ctx) -> TransactionOutput {
        let wp_auth_policy = ctx.select::<MintWPAuthPolicy>().0;
        // BUG! Should call compute_mint_wp_auth_token_policy_id!!!!!
        //let weighting_power_policy = compute_mint_wp_auth_token_policy_id(
        //    self.epoch as u64,
        //    ctx.select::<WPAuthPolicy>().0,
        //    ctx.select::<GTAuthPolicy>().0,
        //);
        //println!(
        //    "WeightingPoll::into_ledger(): weighting_power_policy {}",
        //    weighting_power_policy.to_hex()
        //);
        let datum = create_datum(
            &self,
            ctx.select::<GenesisEpochStartTime>(),
            ctx.select::<WeightingPowerPolicy>().0,
        );

        println!("WEIGHTING_POLL DATUM: {:?}", datum);

        let cred = StakeCredential::new_script(wp_auth_policy);
        let address = EnterpriseAddress::new(ctx.select::<NodeMagic>().0 as u8, cred).to_address();

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
    println!("WP datum:distribution: {:?}", wpoll.distribution);
    println!("WP datum:epoch: {}", wpoll.epoch);
    println!("WP datum:deadline: {}", deadline_inner);
    let deadline = PlutusData::new_integer(BigInteger::from(deadline_inner));
    let emission_rate = PlutusData::new_integer(BigInteger::from(wpoll.emission_rate.untag()));
    println!("WP datum:emission_rate: {}", wpoll.emission_rate.untag());
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

impl Stable for WeightingPoll {
    type StableId = WeightingPollId;
    fn stable_id(&self) -> Self::StableId {
        WeightingPollId(self.epoch)
    }
    fn is_quasi_permanent(&self) -> bool {
        true
    }
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

pub struct PollExhausted;

pub enum PollState {
    WeightingOngoing(WeightingOngoing),
    DistributionOngoing(DistributionOngoing),
    PollExhausted(PollExhausted),
}

impl WeightingPoll {
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
                None => PollState::PollExhausted(PollExhausted),
                Some((farm, weight)) => PollState::DistributionOngoing(DistributionOngoing(farm, weight)),
            }
        }
    }

    pub fn voting_deadline_time(&self, genesis: GenesisEpochStartTime) -> NetworkTime {
        epoch_end(genesis, self.epoch)
    }

    fn weighting_open(&self, genesis: GenesisEpochStartTime, time_now: NetworkTime) -> bool {
        epoch_start(genesis, self.epoch) < time_now && epoch_end(genesis, self.epoch) > time_now
    }

    fn distribution_finished(&self) -> bool {
        self.reserves_splash() == 0
    }
}

impl<C> TryFromLedger<TransactionOutput, C> for WeightingPollSnapshot
where
    C: Has<CurrentEpoch>
        + Has<DeployedScriptInfo<{ ProtocolValidator::MintWpAuthPolicy as u8 }>>
        + Has<OutputRef>,
{
    fn try_from_ledger(repr: &TransactionOutput, ctx: &C) -> Option<Self> {
        if test_address(repr.address(), ctx) {
            println!("FOUND WEIGHTING_POLL ADDRESS");
            let value = repr.value().clone();
            let WeightingPollConfig {
                distribution,
                emission_rate,
                weighting_power_policy,
                ..
            } = WeightingPollConfig::try_from_pd(repr.datum()?.into_pd()?)?;

            let epoch = ctx.select::<CurrentEpoch>().0;
            let distribution = distribution.into_iter().map(|f| (f.id, f.weight)).collect();
            let weighting_power_asset_name = compute_epoch_asset_name(epoch);
            let weighting_power = value
                .multiasset
                .get(&weighting_power_policy, &weighting_power_asset_name);

            let weighting_poll = WeightingPoll {
                epoch,
                distribution,
                emission_rate: TaggedAmount::new(emission_rate),
                weighting_power,
            };
            let output_ref = ctx.select::<OutputRef>();
            return Some(Snapshot::new(weighting_poll, output_ref));
        }
        None
    }
}

fn distribution_to_plutus_data(distribution: &[(FarmId, u64)]) -> PlutusData {
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
        match self {
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
        }
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

pub const MINT_WP_AUTH_EX_UNITS: ExUnits = ExUnits {
    mem: 500_000,
    steps: 200_000_000,
    encodings: None,
};

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
    let params_pd = uplc::PlutusData::Array(vec![
        uplc::PlutusData::BoundedBytes(PlutusBytes::from(splash_policy.to_raw_bytes().to_vec())),
        uplc::PlutusData::BoundedBytes(PlutusBytes::from(farm_auth_policy.to_raw_bytes().to_vec())),
        uplc::PlutusData::BoundedBytes(PlutusBytes::from(factory_auth_policy.to_raw_bytes().to_vec())),
        uplc::PlutusData::BoundedBytes(PlutusBytes::from(
            inflation_box_auth_policy.to_raw_bytes().to_vec(),
        )),
        uplc::PlutusData::BigInt(uplc::BigInt::Int(Int::from(zeroth_epoch_start as i64))),
    ]);
    apply_params_validator(params_pd, MINT_WP_AUTH_TOKEN_SCRIPT)
}
