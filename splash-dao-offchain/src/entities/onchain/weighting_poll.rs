use std::fmt::Formatter;

use cml_chain::plutus::{ConstrPlutusData, PlutusData};
use cml_chain::transaction::TransactionOutput;
use cml_chain::utils::BigInt;
use cml_chain::PolicyId;
use derive_more::From;

use spectrum_cardano_lib::plutus_data::{ConstrPlutusDataExtension, IntoPlutusData, PlutusDataExtension};
use spectrum_cardano_lib::Token;
use spectrum_offchain::data::{EntitySnapshot, Identifier, Stable};
use spectrum_offchain::ledger::IntoLedger;

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
    ) -> Self {
        let stable_id = WeightingPollStableId {
            auth_policy,
            farm_auth_policy,
        };
        Self {
            epoch,
            distribution: farms.into_iter().map(|farm| (farm, 0)).collect(),
            stable_id,
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
