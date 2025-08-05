use std::fmt::{Debug, Display, Formatter};

use cml_chain::address::Address;
use cml_chain::builders::input_builder::SingleInputBuilder;
use cml_chain::builders::output_builder::SingleOutputBuilderResult;
use cml_chain::builders::redeemer_builder::RedeemerWitnessKey;
use cml_chain::builders::tx_builder::{
    ChangeSelectionAlgo, SignedTxBuilder, TransactionUnspentOutput, TxBuilderError,
};
use cml_chain::builders::withdrawal_builder::{SingleWithdrawalBuilder, WithdrawalBuilderError};
use cml_chain::builders::witness_builder::{PartialPlutusWitness, PlutusScriptWitness};
use cml_chain::certs::Credential;
use cml_chain::plutus::{PlutusData, RedeemerTag};
use cml_chain::transaction::{DatumOption, ScriptRef, TransactionOutput};
use cml_chain::utils::BigInteger;
use cml_chain::{Coin, Value};
use log::info;
use void::Void;

use bloom_offchain::execution_engine::bundled::Bundled;
use bloom_offchain::execution_engine::liquidity_book::core::Next;
use bloom_offchain::execution_engine::liquidity_book::market_maker::AvailableLiquidity;
use bloom_offchain::execution_engine::liquidity_book::market_maker::{
    AbsoluteReserves, MakerBehavior, MarketMaker, PoolQuality, SpotPrice,
};
use bloom_offchain::execution_engine::liquidity_book::side::OnSide;
use bloom_offchain::execution_engine::liquidity_book::types::AbsolutePrice;
use spectrum_cardano_lib::collateral::Collateral;
use spectrum_cardano_lib::ex_units::ExUnits;
use spectrum_cardano_lib::hash::hash_transaction_canonical;
use spectrum_cardano_lib::output::FinalizedTxOut;
use spectrum_cardano_lib::protocol_params::constant_tx_builder;
use spectrum_cardano_lib::{AssetClass, NetworkId, OutputRef, TaggedAmount, Token};
use spectrum_offchain::domain::event::Predicted;
use spectrum_offchain::domain::{Has, Stable, Tradable};
use spectrum_offchain::executor::RunOrderError;
use spectrum_offchain::ledger::{IntoLedger, TryFromLedger};

use crate::creds::OperatorRewardAddress;
use crate::data::balance_pool::{BalancePool, BalancePoolRedeemer};
use crate::data::cfmm_pool::{CFMMPoolRedeemer, ConstFnPool};
use crate::data::operation_output::{OperationResultBlueprint, OperationResultOutputs};
use crate::data::order::{ClassicalOrderAction, ClassicalOrderRedeemer, Quote};
use crate::data::pair::PairId;
use crate::data::pool::AnyPool::{BalancedCFMM, PureCFMM, StableCFMM};
use crate::data::stable_pool_t2t::{StablePoolRedeemer, StablePoolT2T as StablePoolT2TData};
use crate::data::OnChainOrderId;
use crate::deployment::ProtocolValidator::{
    BalanceFnPoolV1, BalanceFnPoolV2, ConstFnPoolFeeSwitch, ConstFnPoolFeeSwitchBiDirFee,
    ConstFnPoolFeeSwitchV2, ConstFnPoolV1, ConstFnPoolV2, RoyaltyPoolV1, RoyaltyPoolV1LedgerFixed,
    RoyaltyPoolV2, StableFnPoolT2T,
};
use crate::deployment::{DeployedScriptInfo, RequiresValidator};

pub struct Rx;

pub struct Ry;

pub struct Lq;

pub enum ApplyOrderError<Order> {
    Slippage(Slippage<Order>),
    LowBatcherFee(LowerBatcherFee<Order>),
    Incompatible(Incompatible<Order>),
    VerificationFailed(VerificationFailed<Order>),
}

impl<Order> ApplyOrderError<Order> {
    pub fn incompatible(order: Order) -> Self {
        Self::Incompatible(Incompatible { order })
    }

    pub fn verification_failed(order: Order, description: String) -> Self {
        Self::VerificationFailed(VerificationFailed { order, description })
    }

    pub fn map<F, T1>(self, f: F) -> ApplyOrderError<T1>
    where
        F: FnOnce(Order) -> T1,
    {
        match self {
            ApplyOrderError::Slippage(slippage) => ApplyOrderError::Slippage(slippage.map(f)),
            ApplyOrderError::LowBatcherFee(low_batcher_fee) => {
                ApplyOrderError::LowBatcherFee(low_batcher_fee.map(f))
            }
            ApplyOrderError::Incompatible(math_error) => ApplyOrderError::Incompatible(math_error.map(f)),
            ApplyOrderError::VerificationFailed(verification_error) => {
                ApplyOrderError::VerificationFailed(verification_error.map(f))
            }
        }
    }

    pub fn slippage(
        order: Order,
        quote_amount: TaggedAmount<Quote>,
        expected_amount: TaggedAmount<Quote>,
    ) -> ApplyOrderError<Order> {
        ApplyOrderError::Slippage(Slippage {
            order,
            quote_amount,
            expected_amount,
        })
    }

    pub fn low_batcher_fee(order: Order, batcher_fee: u64, ada_deposit: Coin) -> ApplyOrderError<Order> {
        ApplyOrderError::LowBatcherFee(LowerBatcherFee {
            order,
            batcher_fee,
            ada_deposit,
        })
    }
}

impl<Order> From<ApplyOrderError<Order>> for RunOrderError<Order> {
    fn from(value: ApplyOrderError<Order>) -> RunOrderError<Order> {
        match value {
            ApplyOrderError::Slippage(slippage) => slippage.into(),
            ApplyOrderError::LowBatcherFee(low_batcher_fee) => low_batcher_fee.into(),
            ApplyOrderError::Incompatible(math_error) => math_error.into(),
            ApplyOrderError::VerificationFailed(verifiaction_failed) => verifiaction_failed.into(),
        }
    }
}

#[derive(Debug)]
pub struct Slippage<Order> {
    pub order: Order,
    pub quote_amount: TaggedAmount<Quote>,
    pub expected_amount: TaggedAmount<Quote>,
}

impl<T> Slippage<T> {
    pub fn map<F, T1>(self, f: F) -> Slippage<T1>
    where
        F: FnOnce(T) -> T1,
    {
        Slippage {
            order: f(self.order),
            quote_amount: self.quote_amount,
            expected_amount: self.expected_amount,
        }
    }
}

impl<Order> From<Slippage<Order>> for RunOrderError<Order> {
    fn from(value: Slippage<Order>) -> Self {
        RunOrderError::NonFatal("Price slippage".to_string(), value.order)
    }
}

#[derive(Debug)]
pub struct LowerBatcherFee<Order> {
    order: Order,
    batcher_fee: u64,
    ada_deposit: Coin,
}

impl<T> LowerBatcherFee<T> {
    pub fn map<F, T1>(self, f: F) -> LowerBatcherFee<T1>
    where
        F: FnOnce(T) -> T1,
    {
        LowerBatcherFee {
            order: f(self.order),
            batcher_fee: self.batcher_fee,
            ada_deposit: self.ada_deposit,
        }
    }
}

impl<Order> From<LowerBatcherFee<Order>> for RunOrderError<Order> {
    fn from(value: LowerBatcherFee<Order>) -> Self {
        RunOrderError::NonFatal(
            format!(
                "Lower batcher fee. Batcher fee {}. Ada deposit {}",
                value.batcher_fee, value.ada_deposit
            ),
            value.order,
        )
    }
}

#[derive(Debug)]
pub struct Incompatible<Order> {
    pub order: Order,
}

impl<T> Incompatible<T> {
    pub fn map<F, T1>(self, f: F) -> Incompatible<T1>
    where
        F: FnOnce(T) -> T1,
    {
        Incompatible { order: f(self.order) }
    }
}

impl<Order> From<Incompatible<Order>> for RunOrderError<Order> {
    fn from(value: Incompatible<Order>) -> Self {
        RunOrderError::NonFatal("Math error".to_string(), value.order)
    }
}

#[derive(Debug)]
pub struct VerificationFailed<Order> {
    pub order: Order,
    pub description: String,
}

impl<T> VerificationFailed<T> {
    pub fn map<F, T1>(self, f: F) -> VerificationFailed<T1>
    where
        F: FnOnce(T) -> T1,
    {
        VerificationFailed {
            order: f(self.order),
            description: self.description,
        }
    }
}

impl<Order> From<VerificationFailed<Order>> for RunOrderError<Order> {
    fn from(value: VerificationFailed<Order>) -> Self {
        RunOrderError::Fatal(format!("Verification failed. {}", value.description), value.order)
    }
}

pub enum CFMMPoolAction {
    Swap,
    Deposit,
    Redeem,
    RoyaltyWithdraw,
    DAOAction,
}

impl CFMMPoolAction {
    pub fn to_plutus_data(self) -> PlutusData {
        match self {
            CFMMPoolAction::Swap => PlutusData::Integer(BigInteger::from(2)),
            CFMMPoolAction::Deposit => PlutusData::Integer(BigInteger::from(0)),
            CFMMPoolAction::Redeem => PlutusData::Integer(BigInteger::from(1)),
            CFMMPoolAction::DAOAction => PlutusData::Integer(BigInteger::from(3)),
            CFMMPoolAction::RoyaltyWithdraw => PlutusData::Integer(BigInteger::from(4)),
        }
    }
}

#[derive(Copy, Clone, Eq, PartialEq, Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PoolValidation {
    pub min_n2t_lovelace: u64,
    pub min_t2t_lovelace: u64,
}

#[derive(Debug, Copy, Clone, Eq, PartialEq, derive_more::Display)]
pub enum AnyPool {
    PureCFMM(ConstFnPool),
    BalancedCFMM(BalancePool),
    StableCFMM(StablePoolT2TData),
}

pub struct PoolAssetMapping {
    pub asset_to_deduct_from: AssetClass,
    pub asset_to_add_to: AssetClass,
}

impl MakerBehavior for AnyPool {
    fn swap(mut self, input: OnSide<u64>) -> Next<Self, Void> {
        match self {
            PureCFMM(p) => p.swap(input).map_succ(PureCFMM),
            BalancedCFMM(p) => p.swap(input).map_succ(BalancedCFMM),
            StableCFMM(p) => p.swap(input).map_succ(StableCFMM),
        }
    }
}

impl MarketMaker for AnyPool {
    type U = ExUnits;
    fn static_price(&self) -> SpotPrice {
        match self {
            PureCFMM(p) => p.static_price(),
            BalancedCFMM(p) => p.static_price(),
            StableCFMM(p) => p.static_price(),
        }
    }

    fn real_price(&self, input: OnSide<u64>) -> Option<AbsolutePrice> {
        match self {
            PureCFMM(p) => p.real_price(input),
            BalancedCFMM(p) => p.real_price(input),
            StableCFMM(p) => p.real_price(input),
        }
    }

    fn quality(&self) -> PoolQuality {
        match self {
            PureCFMM(p) => p.quality(),
            BalancedCFMM(p) => p.quality(),
            StableCFMM(p) => p.quality(),
        }
    }

    fn marginal_cost_hint(&self) -> Self::U {
        match self {
            PureCFMM(p) => p.marginal_cost_hint(),
            BalancedCFMM(p) => p.marginal_cost_hint(),
            StableCFMM(p) => p.marginal_cost_hint(),
        }
    }

    fn liquidity(&self) -> AbsoluteReserves {
        match self {
            PureCFMM(p) => p.liquidity(),
            BalancedCFMM(p) => p.liquidity(),
            StableCFMM(p) => p.liquidity(),
        }
    }

    fn available_liquidity_on_side(&self, worst_price: OnSide<AbsolutePrice>) -> Option<AvailableLiquidity> {
        match self {
            PureCFMM(p) => p.available_liquidity_on_side(worst_price),
            BalancedCFMM(p) => p.available_liquidity_on_side(worst_price),
            StableCFMM(p) => p.available_liquidity_on_side(worst_price),
        }
    }

    fn estimated_trade(&self, input: OnSide<u64>) -> Option<AvailableLiquidity> {
        match self {
            PureCFMM(p) => p.estimated_trade(input),
            BalancedCFMM(p) => p.estimated_trade(input),
            StableCFMM(_) => None,
        }
    }

    fn is_active(&self) -> bool {
        match self {
            PureCFMM(p) => p.is_active(),
            BalancedCFMM(p) => p.is_active(),
            StableCFMM(p) => p.is_active(),
        }
    }
}

impl<C> TryFromLedger<TransactionOutput, C> for AnyPool
where
    C: Has<DeployedScriptInfo<{ ConstFnPoolV1 as u8 }>>
        + Has<DeployedScriptInfo<{ ConstFnPoolV2 as u8 }>>
        + Has<DeployedScriptInfo<{ ConstFnPoolFeeSwitch as u8 }>>
        + Has<DeployedScriptInfo<{ ConstFnPoolFeeSwitchV2 as u8 }>>
        + Has<DeployedScriptInfo<{ ConstFnPoolFeeSwitchBiDirFee as u8 }>>
        + Has<DeployedScriptInfo<{ BalanceFnPoolV1 as u8 }>>
        + Has<DeployedScriptInfo<{ BalanceFnPoolV2 as u8 }>>
        + Has<DeployedScriptInfo<{ StableFnPoolT2T as u8 }>>
        + Has<DeployedScriptInfo<{ RoyaltyPoolV1 as u8 }>>
        + Has<DeployedScriptInfo<{ RoyaltyPoolV1LedgerFixed as u8 }>>
        + Has<DeployedScriptInfo<{ RoyaltyPoolV2 as u8 }>>
        + Has<PoolValidation>,
{
    fn try_from_ledger(repr: &TransactionOutput, ctx: &C) -> Option<Self> {
        ConstFnPool::try_from_ledger(repr, ctx)
            .map(PureCFMM)
            .or_else(|| BalancePool::try_from_ledger(repr, ctx).map(BalancedCFMM))
            .or_else(|| StablePoolT2TData::try_from_ledger(repr, ctx).map(StableCFMM))
    }
}

impl Stable for AnyPool {
    type StableId = Token;
    fn stable_id(&self) -> Self::StableId {
        match self {
            PureCFMM(p) => Token::from(p.stable_id()),
            BalancedCFMM(p) => Token::from(p.id),
            StableCFMM(p) => Token::from(p.id),
        }
    }
    fn is_quasi_permanent(&self) -> bool {
        true
    }
}

impl Tradable for AnyPool {
    type PairId = PairId;
    fn pair_id(&self) -> Self::PairId {
        match self {
            PureCFMM(p) => PairId::canonical(p.asset_x().untag(), p.asset_y().untag()),
            BalancedCFMM(p) => PairId::canonical(p.asset_x.untag(), p.asset_y.untag()),
            StableCFMM(p) => PairId::canonical(p.asset_x.untag(), p.asset_y.untag()),
        }
    }
}

pub struct ImmutablePoolUtxo {
    pub address: Address,
    pub value: Coin,
    pub datum_option: Option<DatumOption>,
    pub script_reference: Option<ScriptRef>,
}

impl From<&TransactionOutput> for ImmutablePoolUtxo {
    fn from(out: &TransactionOutput) -> Self {
        Self {
            address: out.address().clone(),
            value: out.amount().coin,
            datum_option: out.datum(),
            script_reference: out.script_ref().cloned(),
        }
    }
}

/// Some on-chain entities may require a redeemer for a specific action.
pub trait RequiresRedeemer<Action> {
    fn redeemer(self, prev_state: Self, pool_input_index: u64, action: Action) -> PlutusData;
}

impl RequiresRedeemer<CFMMPoolAction> for ConstFnPool {
    fn redeemer(self, _: Self, pool_input_index: u64, action: CFMMPoolAction) -> PlutusData {
        CFMMPoolRedeemer {
            pool_input_index,
            action,
        }
        .to_plutus_data()
    }
}

impl RequiresRedeemer<CFMMPoolAction> for BalancePool {
    fn redeemer(self, prev_state: Self, pool_input_index: u64, action: CFMMPoolAction) -> PlutusData {
        BalancePoolRedeemer {
            pool_input_index,
            action,
            new_pool_state: self,
            prev_pool_state: prev_state,
        }
        .to_plutus_data()
    }
}

impl RequiresRedeemer<CFMMPoolAction> for StablePoolT2TData {
    // used for deposit/redeem operations. Pool output index is 0
    fn redeemer(self, prev_state: Self, pool_input_index: u64, action: CFMMPoolAction) -> PlutusData {
        StablePoolRedeemer {
            pool_input_index,
            pool_output_index: 0,
            action,
            new_pool_state: self,
            prev_pool_state: prev_state,
        }
        .to_plutus_data()
    }
}

pub fn from_tx_builder_error<O>(cml_error: TxBuilderError, order: O) -> RunOrderError<O> {
    RunOrderError::Fatal(cml_error.to_string(), order)
}

pub fn from_withdrawal_builder_error<O>(cml_error: WithdrawalBuilderError, order: O) -> RunOrderError<O> {
    RunOrderError::Fatal(cml_error.to_string(), order)
}

pub trait ApplyOrder<Order, Ctx>: Sized {
    type Result;
    /// Returns new pool, order output
    fn apply_order(
        self,
        order: Order,
        ctx: Ctx,
    ) -> Result<(Self, OperationResultBlueprint<Self::Result>), ApplyOrderError<Order>>;
}

fn wrap_cml_action<U, Order>(
    action: Result<U, TxBuilderError>,
    ord: Bundled<Order, FinalizedTxOut>,
) -> Result<U, RunOrderError<Bundled<Order, FinalizedTxOut>>> {
    match action {
        Ok(res) => Ok(res),
        Err(some_err) => Err(RunOrderError::Fatal(format!("Cml error: {:?}", some_err), ord)),
    }
}

pub fn try_run_order_against_pool<Order, Pool, Ctx>(
    pool_bundle: Bundled<Pool, FinalizedTxOut>,
    order_bundle: Bundled<Order, FinalizedTxOut>,
    ctx: Ctx,
) -> Result<
    (SignedTxBuilder, Predicted<Bundled<Pool, FinalizedTxOut>>),
    RunOrderError<Bundled<Order, FinalizedTxOut>>,
>
where
    Pool: ApplyOrder<Order, Ctx>
        + RequiresValidator<Ctx>
        + IntoLedger<TransactionOutput, ImmutablePoolUtxo>
        + RequiresRedeemer<CFMMPoolAction>
        + Copy,
    <Pool as ApplyOrder<Order, Ctx>>::Result: IntoLedger<TransactionOutput, Ctx> + Clone,
    Order: Has<OnChainOrderId> + Clone + Debug,
    Order: Into<CFMMPoolAction>,
    Ctx: Clone + Has<Collateral> + Has<OperatorRewardAddress> + Has<NetworkId>,
{
    let Bundled(pool, FinalizedTxOut(pool_utxo, pool_ref)) = pool_bundle;
    let Bundled(order, FinalizedTxOut(order_utxo, order_ref)) = order_bundle.clone();

    info!(target: "offchain", "Running order {} against pool {}", order_ref, pool_ref);

    let mut sorted_inputs = [pool_ref, order_ref];
    sorted_inputs.sort();

    let (pool_in_idx, order_in_idx) = match sorted_inputs {
        [lh, _] if lh == pool_ref => (0u64, 1u64),
        _ => (1u64, 0u64),
    };

    let immut_pool = ImmutablePoolUtxo::from(&pool_utxo);
    let order_redeemer = ClassicalOrderRedeemer {
        pool_input_index: pool_in_idx,
        order_input_index: order_in_idx,
        output_index: 1,
        action: ClassicalOrderAction::Apply,
    };

    let (next_pool, operation_result_blueprint) = match pool.apply_order(order.clone(), ctx.clone()) {
        Ok(res) => res,
        Err(order_error) => {
            return Err(order_error
                .map(|value| Bundled(value, FinalizedTxOut(order_utxo, order_ref)))
                .into());
        }
    };
    let pool_out = next_pool.into_ledger(immut_pool);

    let pool_validator = pool.get_validator(&ctx);
    let pool_script = PartialPlutusWitness::new(
        PlutusScriptWitness::Ref(pool_validator.hash),
        next_pool
            .clone()
            .redeemer(pool, pool_in_idx, order.clone().into()),
    );

    let pool_in = SingleInputBuilder::new(pool_ref.into(), pool_utxo.clone())
        .plutus_script_inline_datum(pool_script, Vec::new().into())
        .unwrap();

    let order_validator = operation_result_blueprint.order_script_validator;
    let order_script = PartialPlutusWitness::new(
        PlutusScriptWitness::Ref(order_validator.hash),
        order_redeemer.to_plutus_data(),
    );
    let order_in = SingleInputBuilder::new(order_ref.into(), order_utxo.clone())
        .plutus_script_inline_datum(order_script, Vec::new().into())
        .unwrap();

    let mut tx_builder = constant_tx_builder();

    tx_builder
        .add_collateral(ctx.select::<Collateral>().into())
        .map_err(|err| from_tx_builder_error(err, order_bundle.clone()))?;

    tx_builder.add_reference_input(pool_validator.reference_utxo);
    tx_builder.add_reference_input(order_validator.reference_utxo);

    tx_builder
        .add_input(pool_in)
        .map_err(|err| from_tx_builder_error(err, order_bundle.clone()))?;

    tx_builder
        .add_input(order_in)
        .map_err(|err| from_tx_builder_error(err, order_bundle.clone()))?;

    tx_builder.set_exunits(
        RedeemerWitnessKey::new(RedeemerTag::Spend, pool_in_idx.clone().into()),
        pool_validator.ex_budget.into(),
    );
    tx_builder.set_exunits(
        RedeemerWitnessKey::new(RedeemerTag::Spend, order_in_idx.clone().into()),
        order_validator.ex_budget.into(),
    );

    tx_builder
        .add_output(SingleOutputBuilderResult::new(pool_out.clone()))
        .map_err(|err| from_tx_builder_error(err, order_bundle.clone()))?;

    match operation_result_blueprint.outputs {
        OperationResultOutputs::SingleOutput(single_output) => tx_builder.add_output(
            SingleOutputBuilderResult::new(single_output.into_ledger(ctx.clone())),
        ),
        OperationResultOutputs::MultipleOutputs(multiple_outputs) => {
            let mut operation_outputs = multiple_outputs.sub_action_utxos;
            operation_outputs.insert(0, multiple_outputs.action_result_output);
            operation_outputs.iter().try_fold((), |_, op_result| {
                tx_builder.add_output(SingleOutputBuilderResult::new(
                    op_result.clone().into_ledger(ctx.clone()),
                ))
            })
        }
    }
    .map_err(|err| from_tx_builder_error(err, order_bundle.clone()))?;

    if let Some((witness_validator, redeemer_creator)) = operation_result_blueprint.witness_script {
        let real_redeemer = redeemer_creator.compute(pool_in_idx, order_in_idx);

        let reward_address = cml_chain::address::RewardAddress::new(
            ctx.select::<NetworkId>().into(),
            Credential::new_script(witness_validator.hash),
        );
        let partial_witness =
            PartialPlutusWitness::new(PlutusScriptWitness::Ref(witness_validator.hash), real_redeemer);

        let withdrawal_result = SingleWithdrawalBuilder::new(reward_address.clone(), 0)
            .plutus_script(partial_witness, vec![].into())
            .map_err(|err| from_withdrawal_builder_error(err, order_bundle.clone()))?;

        tx_builder.add_reference_input(witness_validator.reference_utxo);

        tx_builder.add_withdrawal(withdrawal_result);

        tx_builder.set_exunits(
            RedeemerWitnessKey::new(RedeemerTag::Reward, 0),
            witness_validator.ex_budget.into(),
        );
    }

    let address: Address = ctx.select::<OperatorRewardAddress>().into();

    /// Temporary solution to handle a potential bug in CML.
    /// The current version produces an incorrect `min_fee`, so some operations
    /// directly set the fee value. This requires manual creation of a batcher fee UTXO.
    ///
    /// TODO: Remove this workaround once the `min_fee` issue is resolved.
    if let Some(strict_fee) = operation_result_blueprint.strict_fee {
        tx_builder.set_fee(strict_fee);
        tx_builder
            .add_output(SingleOutputBuilderResult::new(TransactionOutput::new(
                address.clone(),
                Value::from(
                    tx_builder
                        .get_total_input()
                        .unwrap()
                        .coin
                        .checked_sub(tx_builder.get_total_output().unwrap().coin)
                        .and_then(|without_fee| without_fee.checked_sub(strict_fee))
                        .ok_or(RunOrderError::raw_builder_error(
                            "Insufficient ada value".to_string(),
                            order_bundle.clone(),
                        ))?,
                ),
                None,
                None,
            )))
            .map_err(|err| from_tx_builder_error(err, order_bundle.clone()))?;
    }

    let tx = wrap_cml_action(
        tx_builder.build(ChangeSelectionAlgo::Default, &address),
        Bundled(order, FinalizedTxOut(order_utxo, order_ref)),
    )?;

    let tx_hash = hash_transaction_canonical(&tx.body());

    let next_pool_ref = OutputRef::new(tx_hash, 0);
    let predicted_pool = Predicted(Bundled(next_pool, FinalizedTxOut(pool_out, next_pool_ref)));

    Ok((tx, predicted_pool))
}

/// Reference Script Output for [ConstFnPool] tagged with pool version [Ver].
#[derive(Debug, Clone)]
pub struct CFMMPoolRefScriptOutput<const VER: u8>(pub TransactionUnspentOutput);

#[cfg(test)]
pub mod tests {
    use crate::data::cfmm_pool::classic_pool::ClassicPool;
    use bloom_offchain::execution_engine::liquidity_book::core::{Next, Trans};
    use bloom_offchain::execution_engine::liquidity_book::market_maker::MakerBehavior;
    use bloom_offchain::execution_engine::liquidity_book::side::OnSide;

    #[test]
    fn tlb_amm_pool_canonical_pair_ordering() {
        // This pool's asset order is canonical
        let pool = gen_pool(true);

        // Contains ADA
        let original_reserve_x = pool.reserves_x.untag();
        // Contains token
        let original_reserve_y = pool.reserves_y.untag();
        let ada_qty = 7000000;

        // Test Ask order (sell ADA to buy token)
        let next_pool = pool.swap(OnSide::Ask(ada_qty));
        let trans_0 = Trans::new(pool, next_pool);
        let output_token_0 = trans_0.loss().unwrap().unwrap();
        let Next::Succ(next_pool) = trans_0.result else {
            unreachable!()
        };
        let next_reserve_x = next_pool.reserves_x.untag();
        let next_reserve_y = next_pool.reserves_y.untag();
        assert_eq!(original_reserve_x, next_reserve_x - ada_qty);
        assert_eq!(original_reserve_y, next_reserve_y + output_token_0);

        // Now test Bid order (buy ADA by selling token)
        let next_next_pool = next_pool.swap(OnSide::Bid(output_token_0));
        let trans_1 = Trans::new(next_pool, next_next_pool);
        let output_ada_1 = trans_1.loss().unwrap().unwrap();
        let Next::Succ(final_pool) = trans_1.result else {
            unreachable!()
        };
        println!("final pool ada reserves: {}", final_pool.reserves_x.untag());
        assert_eq!(next_reserve_x, final_pool.reserves_x.untag() + output_ada_1);
        assert_eq!(next_reserve_y, final_pool.reserves_y.untag() - output_token_0);
    }

    #[test]
    fn tlb_amm_pool_non_canonical_pair_ordering() {
        // This pool's asset order is non-canonical
        let pool = gen_pool(false);

        // Contains tokens
        let original_reserve_x = pool.reserves_x.untag();
        // Contains ADA
        let original_reserve_y = pool.reserves_y.untag();
        let qty = 7000000;

        // Test Ask order (sell ADA to buy token)
        let next_pool = pool.swap(OnSide::Ask(qty));
        let trans_0 = Trans::new(pool, next_pool);
        let output_token_0 = trans_0.loss().unwrap().unwrap();
        let Next::Succ(next_pool) = trans_0.result else {
            unreachable!()
        };
        let next_reserve_x = next_pool.reserves_x.untag();
        let next_reserve_y = next_pool.reserves_y.untag();
        println!("next_x: {}, next_y: {}", next_reserve_x, next_reserve_y);
        assert_eq!(original_reserve_y, next_reserve_y - qty);
        assert_eq!(original_reserve_x, next_reserve_x + output_token_0);

        // Now test Bid order (buy ADA by selling token)
        let next_next_pool = next_pool.swap(OnSide::Bid(output_token_0));
        let trans_1 = Trans::new(next_pool, next_next_pool);
        let output_ada_1 = trans_1.loss().unwrap().unwrap();
        let Next::Succ(final_pool) = trans_1.result else {
            unreachable!()
        };
        assert_eq!(next_reserve_y, final_pool.reserves_y.untag() + output_ada_1);
        assert_eq!(next_reserve_x, final_pool.reserves_x.untag() - output_token_0);
    }

    fn gen_pool(ada_first: bool) -> ClassicPool {
        todo!()
    }
}
