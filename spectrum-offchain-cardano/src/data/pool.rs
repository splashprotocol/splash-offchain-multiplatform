use std::fmt::{Debug, Display, Formatter};

use cml_chain::address::Address;
use cml_chain::builders::input_builder::SingleInputBuilder;
use cml_chain::builders::output_builder::SingleOutputBuilderResult;
use cml_chain::builders::redeemer_builder::RedeemerWitnessKey;
use cml_chain::builders::tx_builder::{
    ChangeSelectionAlgo, SignedTxBuilder, TransactionUnspentOutput, TxBuilderError,
};
use cml_chain::builders::witness_builder::{PartialPlutusWitness, PlutusScriptWitness};
use cml_chain::plutus::{PlutusData, RedeemerTag};
use cml_chain::transaction::{DatumOption, ScriptRef, TransactionOutput};
use cml_chain::utils::BigInteger;
use cml_chain::{Coin, PolicyId};
use cml_core::serialization::Serialize;

use cml_multi_era::babbage::BabbageTransactionOutput;
use log::info;

use bloom_offchain::execution_engine::bundled::Bundled;
use bloom_offchain::execution_engine::liquidity_book::core::{MakeInProgress, Next, Unit};
use bloom_offchain::execution_engine::liquidity_book::market_maker::{AbsoluteReserves, MakerBehavior, MarketMaker, PoolQuality, SpotPrice};
use bloom_offchain::execution_engine::liquidity_book::side::Side;
use bloom_offchain::execution_engine::liquidity_book::types::AbsolutePrice;
use spectrum_cardano_lib::collateral::Collateral;
use spectrum_cardano_lib::ex_units::ExUnits;
use spectrum_cardano_lib::hash::hash_transaction_canonical;
use spectrum_cardano_lib::output::FinalizedTxOut;
use spectrum_cardano_lib::protocol_params::constant_tx_builder;
use spectrum_cardano_lib::{AssetClass, OutputRef, TaggedAmount, Token};
use spectrum_offchain::data::event::Predicted;
use spectrum_offchain::data::{Has, Stable, Tradable};
use spectrum_offchain::executor::RunOrderError;
use spectrum_offchain::ledger::{IntoLedger, TryFromLedger};

use crate::creds::OperatorRewardAddress;
use crate::data::balance_pool::{BalancePool, BalancePoolRedeemer};
use crate::data::cfmm_pool::{CFMMPoolRedeemer, ConstFnPool};
use crate::data::order::{ClassicalOrderAction, ClassicalOrderRedeemer, Quote};
use crate::data::pair::PairId;
use crate::data::pool::AnyPool::{BalancedCFMM, PureCFMM, StableCFMM};
use spectrum_cardano_lib::transaction::TransactionOutputExtension;
use spectrum_cardano_lib::value::ValueExtension;

use crate::data::stable_pool_t2t::{StablePoolRedeemer, StablePoolT2T as StablePoolT2TData};
use crate::data::OnChainOrderId;
use crate::deployment::ProtocolValidator::{
    BalanceFnPoolV1, ConstFnPoolFeeSwitch, ConstFnPoolFeeSwitchBiDirFee, ConstFnPoolV1, ConstFnPoolV2,
    StableFnPoolT2T,
};
use crate::deployment::{DeployedScriptInfo, RequiresValidator};

pub struct Rx;

pub struct Ry;

pub struct Lq;

pub enum ApplyOrderError<Order> {
    Slippage(Slippage<Order>),
    LowBatcherFee(LowerBatcherFee<Order>),
    Incompatible(Incompatible<Order>),
}

impl<Order> ApplyOrderError<Order> {
    pub fn incompatible(order: Order) -> Self {
        Self::Incompatible(Incompatible { order })
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

pub enum CFMMPoolAction {
    Swap,
    Deposit,
    Redeem,
    Destroy,
}

impl CFMMPoolAction {
    pub fn to_plutus_data(self) -> PlutusData {
        match self {
            CFMMPoolAction::Swap => PlutusData::Integer(BigInteger::from(2)),
            CFMMPoolAction::Deposit => PlutusData::Integer(BigInteger::from(0)),
            CFMMPoolAction::Redeem => PlutusData::Integer(BigInteger::from(1)),
            CFMMPoolAction::Destroy => PlutusData::Integer(BigInteger::from(3)),
        }
    }
}

#[derive(Copy, Clone, Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PoolBounds {
    pub min_lovelace: u64,
}

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub enum AnyPool {
    PureCFMM(ConstFnPool),
    BalancedCFMM(BalancePool),
    StableCFMM(StablePoolT2TData),
}

impl Display for AnyPool {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            PureCFMM(p) => f.write_str(&*format!(
                "PureCFMM(id: {}, static_price: {}, quality: {})",
                p.id,
                p.static_price(),
                p.quality()
            )),
            BalancedCFMM(p) => f.write_str(&*format!(
                "BalancedCFMM(id: {}, static_price: {}, quality: {})",
                p.id,
                p.static_price(),
                p.quality()
            )),
            StableCFMM(p) => f.write_str(&*format!(
                "StableCFMM(id: {}, static_price: {}, quality: {})",
                p.id,
                p.static_price(),
                p.quality()
            )),
        }
    }
}

pub struct PoolAssetMapping {
    pub asset_to_deduct_from: AssetClass,
    pub asset_to_add_to: AssetClass,
}

impl MakerBehavior for AnyPool {
    fn swap(mut self, input: Side<u64>) -> Next<Self, Unit> {
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

    fn real_price(&self, input: Side<u64>) -> Option<AbsolutePrice> {
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

    fn is_active(&self) -> bool {
        match self {
            PureCFMM(p) => p.is_active(),
            BalancedCFMM(p) => p.is_active(),
            StableCFMM(p) => p.is_active(),
        }
    }
}

impl<C> TryFromLedger<BabbageTransactionOutput, C> for AnyPool
where
    C: Has<DeployedScriptInfo<{ ConstFnPoolV1 as u8 }>>
        + Has<DeployedScriptInfo<{ ConstFnPoolV2 as u8 }>>
        + Has<DeployedScriptInfo<{ ConstFnPoolFeeSwitch as u8 }>>
        + Has<DeployedScriptInfo<{ ConstFnPoolFeeSwitchBiDirFee as u8 }>>
        + Has<DeployedScriptInfo<{ BalanceFnPoolV1 as u8 }>>
        + Has<DeployedScriptInfo<{ StableFnPoolT2T as u8 }>>
        + Has<PoolBounds>,
{
    fn try_from_ledger(repr: &BabbageTransactionOutput, ctx: &C) -> Option<Self> {
        ConstFnPool::try_from_ledger(repr, ctx)
            .map(PureCFMM)
            .or_else(|| BalancePool::try_from_ledger(repr, ctx).map(BalancedCFMM))
            .or_else(|| StablePoolT2TData::try_from_ledger(repr, ctx).map(StableCFMM))
    }
}

impl Stable for AnyPool {
    type StableId = PolicyId;
    fn stable_id(&self) -> Self::StableId {
        match self {
            PureCFMM(p) => Token::from(p.id).0,
            BalancedCFMM(p) => Token::from(p.id).0,
            StableCFMM(p) => Token::from(p.id).0,
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
            PureCFMM(p) => PairId::canonical(p.asset_x.untag(), p.asset_y.untag()),
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

pub trait ApplyOrder<Order>: Sized {
    type Result;

    /// Returns new pool, order output
    fn apply_order(self, order: Order) -> Result<(Self, Self::Result), ApplyOrderError<Order>>;
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
    Pool: ApplyOrder<Order>
        + RequiresValidator<Ctx>
        + IntoLedger<TransactionOutput, ImmutablePoolUtxo>
        + RequiresRedeemer<CFMMPoolAction>
        + Clone,
    <Pool as ApplyOrder<Order>>::Result: IntoLedger<TransactionOutput, Ctx>,
    Order: Has<OnChainOrderId> + RequiresValidator<Ctx> + Clone + Debug,
    Order: Into<CFMMPoolAction>,
    Ctx: Clone + Has<Collateral> + Has<OperatorRewardAddress>,
{
    let Bundled(pool, FinalizedTxOut(pool_utxo, pool_ref)) = pool_bundle.clone();
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
    let order_validator = order.get_validator(&ctx);
    let order_script = PartialPlutusWitness::new(
        PlutusScriptWitness::Ref(order_validator.hash),
        order_redeemer.to_plutus_data(),
    );
    let order_in = SingleInputBuilder::new(order_ref.into(), order_utxo.clone())
        .plutus_script_inline_datum(order_script, Vec::new())
        .unwrap();
    let (next_pool, user_out) = match pool.clone().apply_order(order.clone()) {
        Ok(res) => res,
        Err(order_error) => {
            return Err(order_error
                .map(|value| Bundled(value, FinalizedTxOut(order_utxo, order_ref)))
                .into());
        }
    };
    let pool_out = next_pool.clone().into_ledger(immut_pool);

    let pool_validator = pool.get_validator(&ctx);
    let pool_script = PartialPlutusWitness::new(
        PlutusScriptWitness::Ref(pool_validator.hash),
        next_pool
            .clone()
            .redeemer(pool.clone(), pool_in_idx, order.clone().into()),
    );

    let pool_in = SingleInputBuilder::new(pool_ref.into(), pool_utxo.clone())
        .plutus_script_inline_datum(pool_script, Vec::new())
        .unwrap();

    let mut tx_builder = constant_tx_builder();

    tx_builder
        .add_collateral(ctx.select::<Collateral>().into())
        .map_err(|err| RunOrderError::from_cml_error(err, order_bundle.clone()))?;

    tx_builder.add_reference_input(order_validator.reference_utxo);
    tx_builder.add_reference_input(pool_validator.reference_utxo);

    tx_builder
        .add_input(pool_in)
        .map_err(|err| RunOrderError::from_cml_error(err, order_bundle.clone()))?;
    tx_builder
        .add_input(order_in)
        .map_err(|err| RunOrderError::from_cml_error(err, order_bundle.clone()))?;

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
        .map_err(|err| RunOrderError::from_cml_error(err, order_bundle.clone()))?;

    tx_builder
        .add_output(SingleOutputBuilderResult::new(user_out.into_ledger(ctx.clone())))
        .map_err(|err| RunOrderError::from_cml_error(err, order_bundle.clone()))?;

    let tx = wrap_cml_action(
        tx_builder.build(
            ChangeSelectionAlgo::Default,
            &ctx.select::<OperatorRewardAddress>().into(),
        ),
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
    use bloom_offchain::execution_engine::liquidity_book::core::{Next, Trans};
    use bloom_offchain::execution_engine::liquidity_book::market_maker::MakerBehavior;
    use bloom_offchain::execution_engine::liquidity_book::side::Side;

    use super::ConstFnPool;

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
        let next_pool = pool.swap(Side::Ask(ada_qty));
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
        let next_next_pool = next_pool.swap(Side::Bid(output_token_0));
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
        let next_pool = pool.swap(Side::Ask(qty));
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
        let next_next_pool = next_pool.swap(Side::Bid(output_token_0));
        let trans_1 = Trans::new(next_pool, next_next_pool);
        let output_ada_1 = trans_1.loss().unwrap().unwrap();
        let Next::Succ(final_pool) = trans_1.result else {
            unreachable!()
        };
        assert_eq!(next_reserve_y, final_pool.reserves_y.untag() + output_ada_1);
        assert_eq!(next_reserve_x, final_pool.reserves_x.untag() - output_token_0);
    }

    fn gen_pool(ada_first: bool) -> ConstFnPool {
        todo!()
    }
}
