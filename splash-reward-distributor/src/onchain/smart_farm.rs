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
    routines::{Slot, TimedOutputRef},
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
        + Has<DeployedScriptInfo<{ DaoProtocolValidator::SmartFarm as u8 }>>,
{
    fn try_from_ledger(repr: &TxViewPartiallyResolved, ctx: &Cx) -> Option<Self> {
        let slot = Slot(repr.slot);
        let created = repr.outputs.iter().enumerate().find_map(|(ix, output)| {
            let output_ref = TimedOutputRef::new(OutputRef::new(repr.hash, ix as u64), slot);
            try_extract_gauge(output, output_ref, ctx).map(|gauge| (gauge, output.clone()))
        })?;
        let consumed = repr.inputs.iter().find_map(|(tx_input, output)| {
            if let Some(output) = output {
                let output_ref = TimedOutputRef::new(OutputRef::from(tx_input.clone()), slot);
                if try_extract_gauge(output, output_ref, ctx).is_some() {
                    return Some(output_ref.output_ref);
                }
            }
            None
        });
        Some(EntityUpdated { consumed, created })
    }
}

fn try_extract_gauge<C>(
    output: &TransactionOutput,
    timed_output_ref: TimedOutputRef,
    ctx: &C,
) -> Option<Gauge<FarmId, OutputRef>>
where
    C: Has<PermManagerAuthPolicy>
        + Has<FarmAuthPolicy>
        + Has<DeployedScriptInfo<{ DaoProtocolValidator::SmartFarm as u8 }>>,
{
    let ctx = GaugeCtx {
        perm_manager_auth_policy: ctx.select::<PermManagerAuthPolicy>(),
        farm_auth_policy: ctx.select::<FarmAuthPolicy>(),
        timed_output_ref,
        deployed_script_info: ctx.select::<DeployedScriptInfo<{ DaoProtocolValidator::SmartFarm as u8 }>>(),
    };
    let snapshot = SmartFarmSnapshot::try_from_ledger(output, &ctx)?;
    let smart_farm = snapshot.get();
    Some(Gauge {
        id: smart_farm.farm_id,
        state_id: timed_output_ref.output_ref,
        balance: todo!(),
    })
}

/// Need this struct simply to use `SmartFarmSnapshot::try_from_ledger(...)` above.
struct GaugeCtx {
    perm_manager_auth_policy: PermManagerAuthPolicy,
    farm_auth_policy: FarmAuthPolicy,
    timed_output_ref: TimedOutputRef,
    deployed_script_info: DeployedScriptInfo<{ DaoProtocolValidator::SmartFarm as u8 }>,
}

impl Has<PermManagerAuthPolicy> for GaugeCtx {
    fn select<U: type_equalities::IsEqual<PermManagerAuthPolicy>>(&self) -> PermManagerAuthPolicy {
        self.perm_manager_auth_policy.clone()
    }
}

impl Has<FarmAuthPolicy> for GaugeCtx {
    fn select<U: type_equalities::IsEqual<FarmAuthPolicy>>(&self) -> FarmAuthPolicy {
        self.farm_auth_policy.clone()
    }
}

impl Has<TimedOutputRef> for GaugeCtx {
    fn select<U: type_equalities::IsEqual<TimedOutputRef>>(&self) -> TimedOutputRef {
        self.timed_output_ref
    }
}

impl Has<DeployedScriptInfo<{ DaoProtocolValidator::SmartFarm as u8 }>> for GaugeCtx {
    fn select<U: type_equalities::IsEqual<DeployedScriptInfo<{ DaoProtocolValidator::SmartFarm as u8 }>>>(
        &self,
    ) -> DeployedScriptInfo<{ DaoProtocolValidator::SmartFarm as u8 }> {
        self.deployed_script_info
    }
}
