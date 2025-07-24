use std::fmt::Display;
use std::hash::Hash;

use cml_chain::transaction::TransactionOutput;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use spectrum_cardano_lib::{tx_view::TxViewPartiallyResolved, OutputRef};
use spectrum_offchain::{
    domain::{EntitySnapshot, Has, Stable},
    ledger::TryFromLedger,
};
use spectrum_offchain_cardano::deployment::DeployedScriptInfo;
use splash_dao_offchain::{
    deployment::ProtocolValidator as DaoProtocolValidator,
    entities::onchain::smart_farm::{FarmId, SmartFarmSnapshot},
    protocol_config::{FarmAuthPolicy, PermManagerAuthPolicy},
    routines::TimedOutputRef,
};

use crate::events::EntityUpdated;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Gauge<GaugeId, StateId> {
    pub id: GaugeId,
    pub state_id: StateId,
    pub balance: u64,
}

impl<GaugeId, StateId> Stable for Gauge<GaugeId, StateId>
where
    GaugeId: Copy + Eq + Hash + Send + Sync + Display,
{
    type StableId = GaugeId;

    fn stable_id(&self) -> Self::StableId {
        self.id
    }

    fn is_quasi_permanent(&self) -> bool {
        true
    }
}

impl<GaugeId, StateId> EntitySnapshot for Gauge<GaugeId, StateId>
where
    GaugeId: Copy + Eq + Hash + Send + Sync + Display,
    StateId: Copy + Eq + Hash + Send + Sync + Display + Serialize + DeserializeOwned,
{
    type Version = StateId;

    fn version(&self) -> Self::Version {
        self.state_id
    }
}

impl<Cx> TryFromLedger<TxViewPartiallyResolved, Cx>
    for EntityUpdated<Gauge<FarmId, OutputRef>, OutputRef, TransactionOutput>
where
    Cx: Has<PermManagerAuthPolicy>
        + Has<FarmAuthPolicy>
        + Has<TimedOutputRef>
        + Has<DeployedScriptInfo<{ DaoProtocolValidator::SmartFarm as u8 }>>,
{
    fn try_from_ledger(repr: &TxViewPartiallyResolved, ctx: &Cx) -> Option<Self> {
        let created = repr.outputs.iter().enumerate().find_map(|(ix, output)| {
            let output_ref = OutputRef::new(repr.hash, ix as u64);
            try_extract_gauge(output, output_ref, ctx).map(|gauge| (gauge, output.clone()))
        })?;
        let consumed = repr.inputs.iter().find_map(|(tx_input, output)| {
            if let Some(output) = output {
                let output_ref = OutputRef::from(tx_input.clone());
                if try_extract_gauge(output, output_ref, ctx).is_some() {
                    return Some(output_ref);
                }
            }
            None
        });
        Some(EntityUpdated { consumed, created })
    }
}

fn try_extract_gauge<C>(
    output: &TransactionOutput,
    output_ref: OutputRef,
    ctx: &C,
) -> Option<Gauge<FarmId, OutputRef>>
where
    C: Has<PermManagerAuthPolicy>
        + Has<FarmAuthPolicy>
        + Has<TimedOutputRef>
        + Has<DeployedScriptInfo<{ DaoProtocolValidator::SmartFarm as u8 }>>,
{
    let snapshot = SmartFarmSnapshot::try_from_ledger(output, ctx)?;
    let smart_farm = snapshot.get();
    Some(Gauge {
        id: smart_farm.farm_id,
        state_id: output_ref,
        balance: todo!(),
    })
}
