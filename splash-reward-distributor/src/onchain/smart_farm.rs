use std::fmt::Display;
use std::hash::Hash;

use cml_chain::transaction::TransactionOutput;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use spectrum_cardano_lib::{
    output::FinalizedTxOut,
    transaction::TransactionOutputExtension,
    tx_view::{TimedOutput, TxViewPartiallyResolved},
    value::ValueExtension,
    AssetClass, AssetName, OutputRef, Token,
};
use spectrum_offchain::{
    domain::{EntitySnapshot, Has, Stable},
    ledger::TryFromLedger,
};
use spectrum_offchain_cardano::deployment::DeployedScriptInfo;
use splash_dao_offchain::{
    constants::SPLASH_NAME,
    deployment::ProtocolValidator as DaoProtocolValidator,
    entities::onchain::smart_farm::{FarmId, SmartFarmSnapshot},
    protocol_config::{FarmAuthPolicy, PermManagerAuthPolicy, SplashPolicy},
    routines::{Slot, TimedOutputRef},
};

use crate::events::EntityUpdated;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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

#[derive(derive_more::From, Clone, Debug, PartialEq, Eq)]
pub struct UpdatedGauges<FarmId, StateId, Bearer>(
    pub Vec<EntityUpdated<Gauge<FarmId, StateId>, StateId, Bearer>>,
);

impl<Cx> TryFromLedger<TxViewPartiallyResolved, Cx> for UpdatedGauges<FarmId, OutputRef, FinalizedTxOut>
where
    Cx: Has<PermManagerAuthPolicy>
        + Has<FarmAuthPolicy>
        + Has<SplashPolicy>
        + Has<DeployedScriptInfo<{ DaoProtocolValidator::SmartFarm as u8 }>>,
{
    fn try_from_ledger(repr: &TxViewPartiallyResolved, ctx: &Cx) -> Option<Self> {
        let slot = Slot(repr.slot);
        let mut successor_ix = 1_u64;

        let consumed_gauges: Vec<_> = repr
            .inputs
            .iter()
            .enumerate()
            .filter_map(|(ix, (_, output))| {
                let output_ref = TimedOutputRef::new(OutputRef::new(repr.hash, ix as u64), slot);
                if let Some(TimedOutput { output, .. }) = output {
                    return try_extract_gauge(output, output_ref, ctx).map(|gauge| {
                        successor_ix += 1;
                        (gauge, successor_ix - 1)
                    });
                }
                None
            })
            .collect();

        let num_consumed_gauges = consumed_gauges.len();

        // `outputs[0]`` contains buffer_wallet_output, `outputs.last` contains change UTxO, the rest
        // are gauge outputs.
        if num_consumed_gauges > 0 && repr.outputs.len() == num_consumed_gauges + 2 {
            let mut res: Vec<EntityUpdated<Gauge<FarmId, OutputRef>, OutputRef, FinalizedTxOut>> = vec![];
            for ((gauge_in, successor_ix), (output_ix, tx_output)) in consumed_gauges
                .into_iter()
                .zip(repr.outputs.iter().enumerate().skip(1).take(num_consumed_gauges))
            {
                if successor_ix != output_ix as u64 {
                    return None;
                }
                let output_ref = TimedOutputRef::new(OutputRef::new(repr.hash, successor_ix), slot);
                if let Some(gauge_out) = try_extract_gauge(tx_output, output_ref, ctx) {
                    if gauge_out.id == gauge_in.id {
                        res.push(EntityUpdated {
                            consumed: Some(gauge_in.state_id),
                            created: (
                                gauge_out,
                                FinalizedTxOut(tx_output.clone(), output_ref.output_ref),
                            ),
                        });
                    }
                } else {
                    return None;
                }
            }
            return Some(res.into());
        }
        None
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
        + Has<SplashPolicy>
        + Has<DeployedScriptInfo<{ DaoProtocolValidator::SmartFarm as u8 }>>,
{
    let splash_policy = ctx.select::<SplashPolicy>().0;
    let ctx = GaugeCtx {
        perm_manager_auth_policy: ctx.select::<PermManagerAuthPolicy>(),
        farm_auth_policy: ctx.select::<FarmAuthPolicy>(),
        timed_output_ref,
        deployed_script_info: ctx.select::<DeployedScriptInfo<{ DaoProtocolValidator::SmartFarm as u8 }>>(),
    };
    let snapshot = SmartFarmSnapshot::try_from_ledger(output, &ctx)?;
    let smart_farm = snapshot.get();
    let splash_asset_class =
        AssetClass::Token(Token(splash_policy, AssetName::from_utf8(SPLASH_NAME.into())));
    let balance = output.value().amount_of(splash_asset_class)?;
    Some(Gauge {
        id: smart_farm.farm_id,
        state_id: timed_output_ref.output_ref,
        balance,
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
