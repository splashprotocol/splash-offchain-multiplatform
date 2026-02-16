use crate::entities::{
    buffer_wallet::try_extract_buffer_wallet, BufferWalletSplashBalanceChange,
    BufferWalletSplashTokenDecrease, BufferWalletSplashTokenIncrease,
};
use std::fmt::Display;
use std::hash::Hash;

use cml_chain::transaction::{TransactionInput, TransactionOutput};
use cml_crypto::TransactionHash;
use log::trace;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use spectrum_cardano_lib::{
    output::FinalizedTxOut,
    transaction::TransactionOutputExtension,
    tx_view::{TimedOutput, TxViewPartiallyResolved},
    value::ValueExtension,
    AssetClass, AssetName, NetworkId, OutputRef, Token,
};
use spectrum_offchain::{
    domain::{EntitySnapshot, Has, Stable},
    ledger::TryFromLedger,
};
use spectrum_offchain_cardano::deployment::DeployedScriptInfo;
use splash_dao_offchain::{
    constants::SPLASH_NAME,
    deployment::ProtocolValidator as DaoProtocolValidator,
    entities::onchain::{
        farm_factory::FarmFactorySnapshot,
        smart_farm::{FarmId, SmartFarmSnapshot},
        weighting_poll::WeightingPollSnapshot,
    },
    protocol_config::{BufferWalletAuthPolicy, FarmFactoryAuthPolicy, PermManagerAuthPolicy, SplashPolicy},
    routines::{Slot, TimedOutputRef},
    GenesisEpochStartTime,
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
pub struct GaugeCharge<FarmId, StateId, Bearer> {
    pub gauge_update: EntityUpdated<Gauge<FarmId, StateId>, StateId, Bearer>,
    pub splash_increase: BufferWalletSplashTokenIncrease,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GaugeWithdrawals<FarmId, StateId, Bearer>(
    pub  Vec<(
        EntityUpdated<Gauge<FarmId, StateId>, StateId, Bearer>,
        BufferWalletSplashTokenDecrease,
    )>,
);

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UpdatedGauges<FarmId, StateId, Bearer> {
    /// New gauge was created.
    Create(EntityUpdated<Gauge<FarmId, StateId>, StateId, Bearer>),
    /// Gauge was charged with SPLASH by DAO-bot.
    Charge(GaugeCharge<FarmId, StateId, Bearer>),
    /// SPLASH was withdrawn from gauges into buffer-wallet by reward-bot.
    Withdrawals(GaugeWithdrawals<FarmId, StateId, Bearer>),
}

impl<Cx> TryFromLedger<TxViewPartiallyResolved, Cx> for UpdatedGauges<FarmId, OutputRef, FinalizedTxOut>
where
    Cx: Has<PermManagerAuthPolicy>
        + Has<FarmFactoryAuthPolicy>
        + Has<SplashPolicy>
        + Has<NetworkId>
        + Has<GenesisEpochStartTime>
        + Has<BufferWalletAuthPolicy>
        + Has<DeployedScriptInfo<{ DaoProtocolValidator::BufferWallet as u8 }>>
        + Has<DeployedScriptInfo<{ DaoProtocolValidator::FarmFactory as u8 }>>
        + Has<DeployedScriptInfo<{ DaoProtocolValidator::SmartFarm as u8 }>>
        + Has<DeployedScriptInfo<{ DaoProtocolValidator::MintWpAuthPolicy as u8 }>>,
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
        + Has<FarmFactoryAuthPolicy>
        + Has<SplashPolicy>
        + Has<GenesisEpochStartTime>
        + Has<NetworkId>
        + Has<BufferWalletAuthPolicy>
        + Has<DeployedScriptInfo<{ DaoProtocolValidator::BufferWallet as u8 }>>
        + Has<DeployedScriptInfo<{ DaoProtocolValidator::FarmFactory as u8 }>>
        + Has<DeployedScriptInfo<{ DaoProtocolValidator::SmartFarm as u8 }>>
        + Has<DeployedScriptInfo<{ DaoProtocolValidator::MintWpAuthPolicy as u8 }>>,
{
    let mut successor_ix = 1_u64;

    let consumed_gauges: Vec<_> = inputs
        .iter()
        .enumerate()
        .filter_map(|(ix, (input, output))| {
            let output_ref = TimedOutputRef::new(OutputRef::from(input.clone()), slot);
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

    if num_consumed_gauges == 0 {
        // If a new gauge is created in this TX:
        // - output[0] contains resulting farm factory
        // - output[1] contains new gauge

        let farm_factory_ctx = FarmFactoryCtx {
            farm_factory_auth_policy: ctx.select::<FarmFactoryAuthPolicy>(),
            deployed_script_info: ctx
                .select::<DeployedScriptInfo<{ DaoProtocolValidator::FarmFactory as u8 }>>(),
        };

        let farm_factory_output = {
            let output = outputs.first()?;

            FarmFactorySnapshot::try_from_ledger(output, &farm_factory_ctx)?
                .get()
                .clone()
        };

        let farm_factory_input = inputs.iter().find_map(|(_, output)| {
            if let Some(TimedOutput { output, .. }) = output {
                if let Some(farm_factory_snapshot) =
                    FarmFactorySnapshot::try_from_ledger(output, &farm_factory_ctx)
                {
                    let farm_factory = farm_factory_snapshot.get();
                    return Some(farm_factory.clone());
                }
            } else {
                trace!("No output found for factory_farm_input");
            }
            None
        })?;
        let correct_farm_ids = farm_factory_output.last_farm_id == farm_factory_input.last_farm_id + 1;
        let seed_data_matches = farm_factory_output.farm_seed_data == farm_factory_input.farm_seed_data;
        if correct_farm_ids && seed_data_matches {
            let output_ref = TimedOutputRef::new(OutputRef::new(tx_hash, 1), slot);
            let new_gauge = try_extract_gauge(&outputs[1], output_ref, ctx)?;
            return Some(UpdatedGauges::Create(EntityUpdated {
                consumed: None,
                created: (
                    new_gauge,
                    FinalizedTxOut(outputs[1].clone(), output_ref.output_ref),
                ),
            }));
        } else {
            return None;
        }
    }

    let make_wpoll_ctx = |ix: usize| {
        let output_ref = OutputRef::from(inputs[ix].0.clone());
        WeightingPollCtx {
            splash_policy: ctx.select::<SplashPolicy>(),
            timed_output_ref: TimedOutputRef::new(output_ref, slot),
            deployed_script_info: ctx
                .select::<DeployedScriptInfo<{ DaoProtocolValidator::MintWpAuthPolicy as u8 }>>(),
            genesis_epoch_start_time: ctx.select::<GenesisEpochStartTime>(),
            network_id: ctx.select::<NetworkId>(),
        }
    };

    // Check for the case where gauge is charged with SPLASH by DAO-bot. Note: the bot charges one
    // gauge per TX.
    if num_consumed_gauges == 1 {
        for (ix, (_, output)) in inputs.iter().enumerate() {
            let wp_ctx = make_wpoll_ctx(ix);
            if let Some(TimedOutput { ref output, .. }) = output {
                if let Some(wpoll_in_snapshot) = WeightingPollSnapshot::try_from_ledger(output, &wp_ctx) {
                    // wpoll output is at output[0]. NOTE: we're reusing `wp_ctx`, which has the
                    // wrong TimedOutputRef value, but it's fine because we just want to check that
                    // the wpoll is actually in the output with decreased SPLASH.
                    let wpoll_out_snapshot = WeightingPollSnapshot::try_from_ledger(&outputs[0], &wp_ctx)?;
                    let wpoll_in_splash_amt = wpoll_in_snapshot.get().remaining_splash_emission.untag();
                    let wpoll_out_splash_amt = wpoll_out_snapshot.get().remaining_splash_emission.untag();

                    trace!(
                        "wpoll_in_splash_amt: {}, wpoll_out_splash_amt: {}",
                        wpoll_in_splash_amt,
                        wpoll_out_splash_amt
                    );

                    if wpoll_in_splash_amt <= wpoll_out_splash_amt {
                        return None;
                    }

                    let splash_emission_diff = wpoll_in_splash_amt - wpoll_out_splash_amt;

                    // Here gauge output is at output[1]
                    let gauge_output_ref = TimedOutputRef::new(OutputRef::new(tx_hash, 1), slot);
                    trace!("gauge_output_ref: {:?}", gauge_output_ref);
                    let gauge_out = try_extract_gauge(&outputs[1], gauge_output_ref, ctx)?;
                    trace!("gauge_out: {:?}", gauge_out);
                    let balance_change = BufferWalletSplashBalanceChange::from_gauge_diff(
                        consumed_gauges[0].0.balance,
                        gauge_out.balance,
                    );
                    let BufferWalletSplashBalanceChange::Decrease(amount) = balance_change else {
                        return None;
                    };

                    trace!(
                        "amount: {}, splash_emission_diff: {}",
                        amount,
                        splash_emission_diff
                    );

                    if amount != splash_emission_diff {
                        return None;
                    }

                    let splash_increase = BufferWalletSplashTokenIncrease(amount);
                    return Some(UpdatedGauges::Charge(GaugeCharge {
                        gauge_update: EntityUpdated {
                            consumed: Some(consumed_gauges[0].0.state_id),
                            created: (
                                gauge_out,
                                FinalizedTxOut(outputs[1].clone(), gauge_output_ref.output_ref),
                            ),
                        },
                        splash_increase,
                    }));
                }
            }
        }
    }

    // `outputs[0]`` contains buffer_wallet_output, `outputs.last` contains change UTxO, the rest
    // are gauge outputs.
    if outputs.len() == num_consumed_gauges + 2 {
        let dummy_output_ref = OutputRef::new(tx_hash, 0);
        try_extract_buffer_wallet(&outputs[0], dummy_output_ref, ctx)?;

        let mut res = vec![];
        for ((gauge_in, successor_ix), (output_ix, tx_output)) in consumed_gauges
            .into_iter()
            .zip(outputs.iter().enumerate().skip(1).take(num_consumed_gauges))
        {
            if successor_ix != output_ix as u64 {
                trace!("successor_ix != output_ix as u64");
                return None;
            }
            let output_ref = TimedOutputRef::new(OutputRef::new(tx_hash, successor_ix), slot);
            if let Some(gauge_out) = try_extract_gauge(tx_output, output_ref, ctx) {
                let balance_change =
                    BufferWalletSplashBalanceChange::from_gauge_diff(gauge_in.balance, gauge_out.balance);
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
                trace!("No gauge output found for output_ix: {}", output_ix);
                return None;
            }
        }

        let all_withdrawals = res.iter().all(|(_, balance_change)| {
            matches!(balance_change, BufferWalletSplashBalanceChange::Increase(_))
        });
        assert!(all_withdrawals);
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
    let balance = output.value().amount_of(splash_asset_class).unwrap_or_default();
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

struct FarmFactoryCtx {
    farm_factory_auth_policy: FarmFactoryAuthPolicy,
    deployed_script_info: DeployedScriptInfo<{ DaoProtocolValidator::FarmFactory as u8 }>,
}

impl Has<FarmFactoryAuthPolicy> for FarmFactoryCtx {
    fn select<U: type_equalities::IsEqual<FarmFactoryAuthPolicy>>(&self) -> FarmFactoryAuthPolicy {
        self.farm_factory_auth_policy.clone()
    }
}

impl Has<DeployedScriptInfo<{ DaoProtocolValidator::FarmFactory as u8 }>> for FarmFactoryCtx {
    fn select<
        U: type_equalities::IsEqual<DeployedScriptInfo<{ DaoProtocolValidator::FarmFactory as u8 }>>,
    >(
        &self,
    ) -> DeployedScriptInfo<{ DaoProtocolValidator::FarmFactory as u8 }> {
        self.deployed_script_info
    }
}

impl Has<OutputRef> for FarmFactoryCtx {
    fn select<U: type_equalities::IsEqual<OutputRef>>(&self) -> OutputRef {
        // Note: a dummy output ref suffices, as we just want to extract a `FarmFactory` instance
        OutputRef::new(TransactionHash::from([0_u8; 32]), 0)
    }
}

struct WeightingPollCtx {
    splash_policy: SplashPolicy,
    timed_output_ref: TimedOutputRef,
    deployed_script_info: DeployedScriptInfo<{ DaoProtocolValidator::MintWpAuthPolicy as u8 }>,
    genesis_epoch_start_time: GenesisEpochStartTime,
    network_id: NetworkId,
}

impl Has<SplashPolicy> for WeightingPollCtx {
    fn select<U: type_equalities::IsEqual<SplashPolicy>>(&self) -> SplashPolicy {
        self.splash_policy.clone()
    }
}

impl Has<TimedOutputRef> for WeightingPollCtx {
    fn select<U: type_equalities::IsEqual<TimedOutputRef>>(&self) -> TimedOutputRef {
        self.timed_output_ref
    }
}

impl Has<DeployedScriptInfo<{ DaoProtocolValidator::MintWpAuthPolicy as u8 }>> for WeightingPollCtx {
    fn select<
        U: type_equalities::IsEqual<DeployedScriptInfo<{ DaoProtocolValidator::MintWpAuthPolicy as u8 }>>,
    >(
        &self,
    ) -> DeployedScriptInfo<{ DaoProtocolValidator::MintWpAuthPolicy as u8 }> {
        self.deployed_script_info
    }
}

impl Has<GenesisEpochStartTime> for WeightingPollCtx {
    fn select<U: type_equalities::IsEqual<GenesisEpochStartTime>>(&self) -> GenesisEpochStartTime {
        self.genesis_epoch_start_time
    }
}

impl Has<NetworkId> for WeightingPollCtx {
    fn select<U: type_equalities::IsEqual<NetworkId>>(&self) -> NetworkId {
        self.network_id
    }
}
