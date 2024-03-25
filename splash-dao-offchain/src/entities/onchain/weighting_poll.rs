use std::fmt::Formatter;

use cml_chain::plutus::{ConstrPlutusData, ExUnits, PlutusData};
use cml_chain::transaction::TransactionOutput;
use cml_chain::utils::BigInt;
use cml_chain::PolicyId;
use cml_crypto::RawBytesEncoding;
use derive_more::From;

use spectrum_cardano_lib::plutus_data::{ConstrPlutusDataExtension, IntoPlutusData, PlutusDataExtension};
use spectrum_cardano_lib::{TaggedAmount, Token};
use spectrum_offchain::data::{Identifier, Stable};
use spectrum_offchain::ledger::IntoLedger;
use spectrum_offchain_cardano::parametrized_validators::apply_params_validator;
use uplc_pallas_codec::utils::{Int, PlutusBytes};

use crate::assets::Splash;
use crate::constants::MINT_WP_AUTH_TOKEN_SCRIPT;
use crate::entities::onchain::smart_farm::FarmId;
use crate::routines::inflation::WeightingPollSnapshot;
use crate::time::{epoch_end, epoch_start, NetworkTime, ProtocolEpoch};
use crate::GenesisEpochStartTime;

#[derive(Copy, Clone, PartialEq, Eq, Ord, PartialOrd, From)]
pub struct WeightingPollId(Token);

impl Identifier for WeightingPollId {
    type For = WeightingPollSnapshot;
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct WeightingPoll {
    pub epoch: ProtocolEpoch,
    pub distribution: Vec<(FarmId, u64)>,
    pub stable_id: WeightingPollStableId,
    pub emission_rate: TaggedAmount<Splash>,
    /// Note: weighting power is not determined until vote stage.
    pub weighting_power: Option<u64>,
}

impl<Ctx> IntoLedger<TransactionOutput, Ctx> for WeightingPoll {
    fn into_ledger(self, ctx: Ctx) -> TransactionOutput {
        todo!()
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct WeightingPollStableId {
    /// The validator will ensure preservation of a token = (`auth_policy`, `binder`).
    pub auth_policy: PolicyId,
    /// The validator will look for a token = (`farm_auth_policy`, `farm_id`) to authorize withdrawal to a `farm_id`.
    pub farm_auth_policy: PolicyId,
}

impl std::fmt::Display for WeightingPollStableId {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(&format!(
            "WeightingPollStableId: auth_policy: {}, farm_auth_policy: {}",
            self.auth_policy, self.farm_auth_policy
        ))
    }
}

impl Stable for WeightingPoll {
    type StableId = WeightingPollStableId;
    fn stable_id(&self) -> Self::StableId {
        self.stable_id
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
    pub fn new(
        epoch: ProtocolEpoch,
        farms: Vec<FarmId>,
        auth_policy: PolicyId,
        farm_auth_policy: PolicyId,
        emission_rate: TaggedAmount<Splash>,
    ) -> Self {
        let stable_id = WeightingPollStableId {
            auth_policy,
            farm_auth_policy,
        };
        Self {
            epoch,
            distribution: farms.into_iter().map(|farm| (farm, 0)).collect(),
            stable_id,
            emission_rate,
            weighting_power: None,
        }
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

fn distribution_to_plutus_data(distribution: &[(FarmId, u64)]) -> PlutusData {
    let mut list = vec![];
    for (farm_id, weight) in distribution {
        list.push(PlutusData::new_list(vec![
            farm_id.into_pd(),
            PlutusData::new_integer(BigInt::from(*weight)),
        ]));
    }
    PlutusData::new_list(list)
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
                        PlutusData::Integer(BigInt::from(farm_ix)),
                        PlutusData::Integer(BigInt::from(farm_in_ix)),
                    ],
                ))
            }
            PollAction::Destroy => PlutusData::ConstrPlutusData(ConstrPlutusData::new(2, vec![])),
        }
    }
}

pub enum MintAction {
    MintAuthToken { factory_in_ix: u32 },
    BurnAuthToken,
}

impl IntoPlutusData for MintAction {
    fn into_pd(self) -> PlutusData {
        match self {
            MintAction::MintAuthToken { factory_in_ix } => PlutusData::ConstrPlutusData(
                ConstrPlutusData::new(0, vec![PlutusData::Integer(BigInt::from(factory_in_ix))]),
            ),
            MintAction::BurnAuthToken => PlutusData::ConstrPlutusData(ConstrPlutusData::new(1, vec![])),
        }
    }
}

pub const MINT_WP_AUTH_EX_UNITS: ExUnits = ExUnits {
    mem: 500_000,
    steps: 200_000_000,
    encodings: None,
};

/// Note that the this is a multivalidator, and can serve as the script that guards the
/// weighting_poll.
pub fn compute_mint_wp_auth_token_policy_id(
    splash_policy: PolicyId,
    farm_auth_policy: PolicyId,
    factory_auth_policy: PolicyId,
    zeroth_epoch_start: u64,
) -> PolicyId {
    let params_pd = uplc::PlutusData::Array(vec![
        uplc::PlutusData::BoundedBytes(PlutusBytes::from(splash_policy.to_raw_bytes().to_vec())),
        uplc::PlutusData::BoundedBytes(PlutusBytes::from(farm_auth_policy.to_raw_bytes().to_vec())),
        uplc::PlutusData::BoundedBytes(PlutusBytes::from(factory_auth_policy.to_raw_bytes().to_vec())),
        uplc::PlutusData::BigInt(uplc::BigInt::Int(Int::from(zeroth_epoch_start as i64))),
    ]);
    apply_params_validator(params_pd, MINT_WP_AUTH_TOKEN_SCRIPT)
}
