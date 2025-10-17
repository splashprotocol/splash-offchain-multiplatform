use crate::entities::{
    BufferWalletSplashBalanceChange, BufferWalletSplashTokenDecrease, BufferWalletSplashTokenIncrease,
};
use std::fmt::Display;
use std::hash::Hash;

use cml_chain::transaction::{TransactionInput, TransactionOutput};
use cml_crypto::TransactionHash;
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
    protocol_config::{PermManagerAuthPolicy, SplashPolicy},
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GaugeDeposits<FarmId, StateId, Bearer>(
    pub  Vec<(
        EntityUpdated<Gauge<FarmId, StateId>, StateId, Bearer>,
        BufferWalletSplashTokenIncrease,
    )>,
);
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GaugeWithdrawals<FarmId, StateId, Bearer>(
    pub  Vec<(
        EntityUpdated<Gauge<FarmId, StateId>, StateId, Bearer>,
        BufferWalletSplashTokenDecrease,
    )>,
);

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UpdatedGauges<FarmId, StateId, Bearer> {
    Deposits(GaugeDeposits<FarmId, StateId, Bearer>),
    Withdrawals(GaugeWithdrawals<FarmId, StateId, Bearer>),
}

impl<Cx> TryFromLedger<TxViewPartiallyResolved, Cx> for UpdatedGauges<FarmId, OutputRef, FinalizedTxOut>
where
    Cx: Has<PermManagerAuthPolicy>
        + Has<SplashPolicy>
        + Has<DeployedScriptInfo<{ DaoProtocolValidator::SmartFarm as u8 }>>,
{
    fn try_from_ledger(repr: &TxViewPartiallyResolved, ctx: &Cx) -> Option<Self> {
        let slot = Slot(repr.slot);
        try_extract_updated_gauges(slot, &repr.inputs, &repr.outputs, repr.hash, ctx)
    }
}

pub fn try_extract_updated_gauges<C>(
    slot: Slot,
    inputs: &[(TransactionInput, Option<TimedOutput>)],
    outputs: &[TransactionOutput],
    tx_hash: TransactionHash,
    ctx: &C,
) -> Option<UpdatedGauges<FarmId, OutputRef, FinalizedTxOut>>
where
    C: Has<PermManagerAuthPolicy>
        + Has<SplashPolicy>
        + Has<DeployedScriptInfo<{ DaoProtocolValidator::SmartFarm as u8 }>>,
{
    let mut successor_ix = 1_u64;

    let consumed_gauges: Vec<_> = inputs
        .iter()
        .enumerate()
        .filter_map(|(ix, (_, output))| {
            let output_ref = TimedOutputRef::new(OutputRef::new(tx_hash, ix as u64), slot);
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
    if num_consumed_gauges > 0 && outputs.len() == num_consumed_gauges + 2 {
        let mut res = vec![];
        for ((gauge_in, successor_ix), (output_ix, tx_output)) in consumed_gauges
            .into_iter()
            .zip(outputs.iter().enumerate().skip(1).take(num_consumed_gauges))
        {
            if successor_ix != output_ix as u64 {
                return None;
            }
            let output_ref = TimedOutputRef::new(OutputRef::new(tx_hash, successor_ix), slot);
            if let Some(gauge_out) = try_extract_gauge(tx_output, output_ref, ctx) {
                let balance_change =
                    BufferWalletSplashBalanceChange::from_diff(gauge_in.balance, gauge_out.balance);
                if gauge_out.id == gauge_in.id {
                    res.push((
                        EntityUpdated {
                            consumed: Some(gauge_in.state_id),
                            created: (
                                gauge_out,
                                FinalizedTxOut(tx_output.clone(), output_ref.output_ref),
                            ),
                        },
                        balance_change,
                    ));
                }
            } else {
                return None;
            }
        }

        let all_deposits = res.iter().all(|(_, balance_change)| {
            matches!(balance_change, BufferWalletSplashBalanceChange::Increase(_))
        });
        let all_withdrawals = res.iter().all(|(_, balance_change)| {
            matches!(balance_change, BufferWalletSplashBalanceChange::Decrease(_))
        });
        if all_deposits {
            let res = res
                .into_iter()
                .map(|(entity_updated, balance_change)| {
                    (
                        entity_updated,
                        BufferWalletSplashTokenIncrease(balance_change.amount()),
                    )
                })
                .collect();
            return Some(UpdatedGauges::Deposits(GaugeDeposits(res)));
        } else if all_withdrawals {
            let res = res
                .into_iter()
                .map(|(entity_updated, balance_change)| {
                    (
                        entity_updated,
                        BufferWalletSplashTokenDecrease(balance_change.amount()),
                    )
                })
                .collect();
            return Some(UpdatedGauges::Withdrawals(GaugeWithdrawals(res)));
        } else {
            panic!("Gauge updates are not all deposits or all withdrawals");
        }
    }
    None
}

pub fn try_extract_gauge<C>(
    output: &TransactionOutput,
    timed_output_ref: TimedOutputRef,
    ctx: &C,
) -> Option<Gauge<FarmId, OutputRef>>
where
    C: Has<PermManagerAuthPolicy>
        + Has<SplashPolicy>
        + Has<DeployedScriptInfo<{ DaoProtocolValidator::SmartFarm as u8 }>>,
{
    let splash_policy = ctx.select::<SplashPolicy>().0;
    let ctx = GaugeCtx {
        perm_manager_auth_policy: ctx.select::<PermManagerAuthPolicy>(),
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
    timed_output_ref: TimedOutputRef,
    deployed_script_info: DeployedScriptInfo<{ DaoProtocolValidator::SmartFarm as u8 }>,
}

impl Has<PermManagerAuthPolicy> for GaugeCtx {
    fn select<U: type_equalities::IsEqual<PermManagerAuthPolicy>>(&self) -> PermManagerAuthPolicy {
        self.perm_manager_auth_policy.clone()
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
