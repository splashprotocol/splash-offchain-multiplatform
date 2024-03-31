use std::fmt::Debug;

use cml_chain::address::Address;

use cml_chain::builders::input_builder::SingleInputBuilder;
use cml_chain::builders::output_builder::SingleOutputBuilderResult;
use cml_chain::builders::redeemer_builder::RedeemerWitnessKey;
use cml_chain::builders::tx_builder::{
    ChangeSelectionAlgo, SignedTxBuilder, TransactionUnspentOutput, TxBuilderError,
};
use cml_chain::builders::witness_builder::{PartialPlutusWitness, PlutusScriptWitness};
use cml_chain::plutus::PlutusData::Integer;
use cml_chain::plutus::{ConstrPlutusData, PlutusData, RedeemerTag};
use cml_chain::transaction::{DatumOption, ScriptRef, TransactionOutput};
use cml_chain::utils::BigInteger;

use cml_chain::{Coin, PolicyId};

use cml_multi_era::babbage::BabbageTransactionOutput;
use log::info;

use bloom_offchain::execution_engine::bundled::Bundled;
use bloom_offchain::execution_engine::liquidity_book::pool::{Pool, PoolQuality};
use bloom_offchain::execution_engine::liquidity_book::side::Side;
use bloom_offchain::execution_engine::liquidity_book::types::AbsolutePrice;
use spectrum_cardano_lib::collateral::Collateral;
use spectrum_cardano_lib::hash::hash_transaction_canonical;
use spectrum_cardano_lib::output::FinalizedTxOut;
use spectrum_cardano_lib::protocol_params::constant_tx_builder;

use crate::creds::OperatorRewardAddress;
use crate::data::balance_pool::BalancePool;
use crate::data::cfmm_pool::{CFMMPoolRedeemer, ConstFnPool};
use crate::data::order::{ClassicalOrderAction, ClassicalOrderRedeemer, Quote};
use crate::data::pair::PairId;
use crate::data::pool::AnyPool::{BalancedCFMM, PureCFMM};
use crate::data::pool::ApplyOrderError::{LowBatcherFeeErr, SlippageErr};
use spectrum_cardano_lib::{AssetClass, OutputRef, TaggedAmount, Token};
use spectrum_offchain::data::unique_entity::Predicted;
use spectrum_offchain::data::{Has, Stable, Tradable};
use spectrum_offchain::executor::RunOrderError;
use spectrum_offchain::ledger::{IntoLedger, TryFromLedger};

use crate::data::OnChainOrderId;
use crate::deployment::ProtocolValidator::{
    BalanceFnPoolV1, ConstFnPoolFeeSwitch, ConstFnPoolFeeSwitchBiDirFee, ConstFnPoolV1, ConstFnPoolV2,
};
use crate::deployment::{DeployedScriptHash, RequiresValidator};

pub struct Rx;

pub struct Ry;

pub struct Lq;

pub enum ApplyOrderError<Order> {
    SlippageErr(Slippage<Order>),
    LowBatcherFeeErr(LowerBatcherFee<Order>),
}

impl<Order> ApplyOrderError<Order> {
    pub fn map<F, T1>(self, f: F) -> ApplyOrderError<T1>
    where
        F: FnOnce(Order) -> T1,
    {
        match self {
            SlippageErr(slippage) => SlippageErr(slippage.map(f)),
            LowBatcherFeeErr(low_batcher_fee) => LowBatcherFeeErr(low_batcher_fee.map(f)),
        }
    }

    pub fn slippage(
        order: Order,
        quote_amount: TaggedAmount<Quote>,
        expected_amount: TaggedAmount<Quote>,
    ) -> ApplyOrderError<Order> {
        SlippageErr(Slippage {
            order,
            quote_amount,
            expected_amount,
        })
    }

    pub fn low_batcher_fee(order: Order, batcher_fee: u64, ada_deposit: Coin) -> ApplyOrderError<Order> {
        LowBatcherFeeErr(LowerBatcherFee {
            order,
            batcher_fee,
            ada_deposit,
        })
    }
}

impl<Order> From<ApplyOrderError<Order>> for RunOrderError<Order> {
    fn from(value: ApplyOrderError<Order>) -> RunOrderError<Order> {
        match value {
            SlippageErr(slippage) => slippage.into(),
            LowBatcherFeeErr(low_batcher_fee) => low_batcher_fee.into(),
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

pub enum CFMMPoolAction {
    Swap,
    Deposit,
    Redeem,
    Destroy,
}

impl CFMMPoolAction {
    pub fn to_plutus_data(self) -> PlutusData {
        match self {
            CFMMPoolAction::Swap => PlutusData::ConstrPlutusData(ConstrPlutusData::new(2, Vec::new())),
            CFMMPoolAction::Deposit => PlutusData::ConstrPlutusData(ConstrPlutusData::new(0, Vec::new())),
            CFMMPoolAction::Redeem => PlutusData::ConstrPlutusData(ConstrPlutusData::new(1, Vec::new())),
            CFMMPoolAction::Destroy => PlutusData::ConstrPlutusData(ConstrPlutusData::new(3, Vec::new())),
        }
    }
}

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub enum AnyPool {
    PureCFMM(ConstFnPool),
    BalancedCFMM(BalancePool),
}

pub struct AssetDeltas {
    pub asset_to_deduct_from: AssetClass,
    pub asset_to_add_to: AssetClass,
}

impl Pool for AnyPool {
    fn static_price(&self) -> AbsolutePrice {
        match self {
            PureCFMM(p) => p.static_price(),
            BalancedCFMM(p) => p.static_price(),
        }
    }

    fn real_price(&self, input: Side<u64>) -> AbsolutePrice {
        match self {
            PureCFMM(p) => p.real_price(input),
            BalancedCFMM(p) => p.real_price(input),
        }
    }

    fn swap(self, input: Side<u64>) -> (u64, Self) {
        match self {
            PureCFMM(p) => {
                let (swap_res, new_pool) = p.swap(input);
                (swap_res, PureCFMM(new_pool))
            }
            BalancedCFMM(p) => {
                let (swap_res, new_pool) = p.swap(input);
                (swap_res, BalancedCFMM(new_pool))
            }
        }
    }

    fn quality(&self) -> PoolQuality {
        match self {
            PureCFMM(p) => p.quality(),
            BalancedCFMM(p) => p.quality(),
        }
    }
}

impl<C> TryFromLedger<BabbageTransactionOutput, C> for AnyPool
where
    C: Has<DeployedScriptHash<{ ConstFnPoolV1 as u8 }>>
        + Has<DeployedScriptHash<{ ConstFnPoolV2 as u8 }>>
        + Has<DeployedScriptHash<{ ConstFnPoolFeeSwitch as u8 }>>
        + Has<DeployedScriptHash<{ ConstFnPoolFeeSwitchBiDirFee as u8 }>>
        + Has<DeployedScriptHash<{ BalanceFnPoolV1 as u8 }>>,
{
    fn try_from_ledger(repr: &BabbageTransactionOutput, ctx: &C) -> Option<Self> {
        let cfmm_pool = ConstFnPool::try_from_ledger(repr, ctx).map(PureCFMM);
        let balance_pool = BalancePool::try_from_ledger(repr, ctx).map(BalancedCFMM);
        cfmm_pool.or(balance_pool)
    }
}

impl Stable for AnyPool {
    type StableId = PolicyId;
    fn stable_id(&self) -> Self::StableId {
        match self {
            PureCFMM(p) => Token::from(p.id).0,
            BalancedCFMM(p) => Token::from(p.id).0,
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

pub trait ApplyOrder<Order>: Sized {
    type Result;

    // return: new pool, order output
    fn apply_order(self, order: Order) -> Result<(Self, Self::Result), ApplyOrderError<Order>>;
}

pub fn try_run_order_against_pool<Order, Pool, Ctx>(
    Bundled(pool, FinalizedTxOut(pool_utxo, pool_ref)): Bundled<Pool, FinalizedTxOut>,
    Bundled(order, FinalizedTxOut(order_utxo, order_ref)): Bundled<Order, FinalizedTxOut>,
    ctx: Ctx,
) -> Result<
    (SignedTxBuilder, Predicted<Bundled<Pool, FinalizedTxOut>>),
    RunOrderError<Bundled<Order, FinalizedTxOut>>,
>
where
    Pool:
        ApplyOrder<Order> + RequiresValidator<Ctx> + IntoLedger<TransactionOutput, ImmutablePoolUtxo> + Clone,
    <Pool as ApplyOrder<Order>>::Result: IntoLedger<TransactionOutput, Ctx>,
    Order: Has<OnChainOrderId> + RequiresValidator<Ctx> + Clone + Debug,
    Order: Into<CFMMPoolAction>,
    Ctx: Clone + Has<Collateral> + Has<OperatorRewardAddress>,
{
    info!(target: "offchain", "Running order {} against pool {}", order_ref, pool_ref);

    let mut sorted_inputs = [pool_ref, order_ref];
    sorted_inputs.sort();

    let (pool_in_idx, order_in_idx) = match sorted_inputs {
        [lh, _] if lh == pool_ref => (0u64, 1u64),
        _ => (1u64, 0u64),
    };

    let pool_redeemer = CFMMPoolRedeemer {
        pool_input_index: pool_in_idx,
        action: order.clone().into(),
    };
    let pool_validator = pool.get_validator(&ctx);
    let pool_script = PartialPlutusWitness::new(
        PlutusScriptWitness::Ref(pool_validator.hash),
        pool_redeemer.to_plutus_data(),
    );
    let immut_pool = ImmutablePoolUtxo::from(&pool_utxo);
    let pool_in = SingleInputBuilder::new(pool_ref.into(), pool_utxo.clone())
        .plutus_script_inline_datum(pool_script, Vec::new())
        .unwrap();
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

    let mut tx_builder = constant_tx_builder();

    tx_builder
        .add_collateral(ctx.select::<Collateral>().into())
        .unwrap();

    tx_builder.add_reference_input(order_validator.reference_utxo);
    tx_builder.add_reference_input(pool_validator.reference_utxo);

    tx_builder.add_input(pool_in).unwrap();
    tx_builder.add_input(order_in).unwrap();

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
        .unwrap();

    tx_builder
        .add_output(SingleOutputBuilderResult::new(user_out.into_ledger(ctx.clone())))
        .unwrap();

    let tx = tx_builder
        .build(
            ChangeSelectionAlgo::Default,
            &ctx.select::<OperatorRewardAddress>().into(),
        )
        .unwrap();

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
    use cml_crypto::TransactionHash;
    use rand::Rng;

    use bloom_offchain::execution_engine::liquidity_book::{pool::Pool, side::Side};
    use spectrum_cardano_lib::OutputRef;
    use spectrum_offchain::ledger::TryFromLedger;

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
        let (output_token_0, next_pool) = pool.swap(Side::Ask(ada_qty));
        let next_reserve_x = next_pool.reserves_x.untag();
        let next_reserve_y = next_pool.reserves_y.untag();
        assert_eq!(original_reserve_x, next_reserve_x - ada_qty);
        assert_eq!(original_reserve_y, next_reserve_y + output_token_0);

        // Now test Bid order (buy ADA by selling token)
        let (output_ada_1, final_pool) = next_pool.swap(Side::Bid(output_token_0));
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
        let (output_token_0, next_pool) = pool.swap(Side::Ask(qty));
        let next_reserve_x = next_pool.reserves_x.untag();
        let next_reserve_y = next_pool.reserves_y.untag();
        println!("next_x: {}, next_y: {}", next_reserve_x, next_reserve_y);
        assert_eq!(original_reserve_y, next_reserve_y - qty);
        assert_eq!(original_reserve_x, next_reserve_x + output_token_0);

        // Now test Bid order (buy ADA by selling token)
        let (output_ada_1, final_pool) = next_pool.swap(Side::Bid(output_token_0));
        assert_eq!(next_reserve_y, final_pool.reserves_y.untag() + output_ada_1);
        assert_eq!(next_reserve_x, final_pool.reserves_x.untag() - output_token_0);
    }

    fn gen_pool(ada_first: bool) -> ConstFnPool {
        todo!()
    }
}
