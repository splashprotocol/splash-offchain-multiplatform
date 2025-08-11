use std::fmt::Display;
use std::hash::Hash;

use cml_chain::transaction::TransactionOutput;
use derive_more::From;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use spectrum_cardano_lib::{
    tx_view::{TimedOutput, TxViewPartiallyResolved},
    OutputRef,
};
use spectrum_offchain::{
    domain::{EntitySnapshot, Has, Stable},
    ledger::TryFromLedger,
};
use spectrum_offchain_cardano::deployment::DeployedScriptInfo;
use splash_dao_offchain::{
    deployment::ProtocolValidator as DaoProtocolValidator,
    entities::onchain::{permission_manager::PermManagerSnapshot, smart_farm::FarmId},
    protocol_config::PermManagerAuthPolicy,
    routines::{Slot, TimedOutputRef},
};

use crate::events::EntityUpdated;

#[derive(
    Copy, Clone, PartialEq, Eq, Ord, PartialOrd, From, Serialize, Deserialize, derive_more::Display, Hash,
)]
pub struct AuthManagerId;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthManager<GaugeId, StateId> {
    state_id: StateId,
    suspended_gauges: Vec<GaugeId>,
}

impl<GaugeId, StateId> Stable for AuthManager<GaugeId, StateId> {
    type StableId = AuthManagerId;

    fn stable_id(&self) -> Self::StableId {
        AuthManagerId
    }

    fn is_quasi_permanent(&self) -> bool {
        true
    }
}

impl<GaugeId, StateId> EntitySnapshot for AuthManager<GaugeId, StateId>
where
    StateId: Copy + Eq + Hash + Send + Sync + Display + Serialize + DeserializeOwned,
{
    type Version = StateId;

    fn version(&self) -> Self::Version {
        self.state_id
    }
}

impl<Cx> TryFromLedger<TxViewPartiallyResolved, Cx>
    for EntityUpdated<AuthManager<FarmId, OutputRef>, OutputRef, TransactionOutput>
where
    Cx: Has<PermManagerAuthPolicy> + Has<DeployedScriptInfo<{ DaoProtocolValidator::PermManager as u8 }>>,
{
    fn try_from_ledger(repr: &TxViewPartiallyResolved, ctx: &Cx) -> Option<Self> {
        let created = repr.outputs.iter().enumerate().find_map(|(ix, output)| {
            let output_ref = TimedOutputRef::new(OutputRef::new(repr.hash, ix as u64), Slot(repr.slot));
            try_extract_auth_manager(output, output_ref, ctx)
                .map(|auth_manager| (auth_manager, output.clone()))
        })?;
        let consumed = repr.inputs.iter().find_map(|(tx_input, output)| {
            if let Some(TimedOutput { output, .. }) = output {
                let output_ref = TimedOutputRef::new(OutputRef::from(tx_input.clone()), Slot(repr.slot));
                if try_extract_auth_manager(output, output_ref, ctx).is_some() {
                    return Some(output_ref.output_ref);
                }
            }
            None
        });
        Some(EntityUpdated { consumed, created })
    }
}

fn try_extract_auth_manager<C>(
    output: &TransactionOutput,
    timed_output_ref: TimedOutputRef,
    ctx: &C,
) -> Option<AuthManager<FarmId, OutputRef>>
where
    C: Has<PermManagerAuthPolicy> + Has<DeployedScriptInfo<{ DaoProtocolValidator::PermManager as u8 }>>,
{
    let ctx = AuthManagerCtx {
        auth_policy: ctx.select::<PermManagerAuthPolicy>(),
        timed_output_ref,
        deployed_script_info: ctx.select::<DeployedScriptInfo<{ DaoProtocolValidator::PermManager as u8 }>>(),
    };
    let snapshot = PermManagerSnapshot::try_from_ledger(output, &ctx)?;
    let auth_manager = snapshot.get();
    Some(AuthManager {
        state_id: timed_output_ref.output_ref,
        suspended_gauges: auth_manager.datum.suspended_farms.clone(),
    })
}

/// Need this struct simply to use `PermManagerSnapshot::try_from_ledger(...)` above.
struct AuthManagerCtx {
    auth_policy: PermManagerAuthPolicy,
    timed_output_ref: TimedOutputRef,
    deployed_script_info: DeployedScriptInfo<{ DaoProtocolValidator::PermManager as u8 }>,
}

impl Has<PermManagerAuthPolicy> for AuthManagerCtx {
    fn select<U: type_equalities::IsEqual<PermManagerAuthPolicy>>(&self) -> PermManagerAuthPolicy {
        self.auth_policy.clone()
    }
}

impl Has<TimedOutputRef> for AuthManagerCtx {
    fn select<U: type_equalities::IsEqual<TimedOutputRef>>(&self) -> TimedOutputRef {
        self.timed_output_ref
    }
}

impl Has<DeployedScriptInfo<{ DaoProtocolValidator::PermManager as u8 }>> for AuthManagerCtx {
    fn select<
        U: type_equalities::IsEqual<DeployedScriptInfo<{ DaoProtocolValidator::PermManager as u8 }>>,
    >(
        &self,
    ) -> DeployedScriptInfo<{ DaoProtocolValidator::PermManager as u8 }> {
        self.deployed_script_info
    }
}
