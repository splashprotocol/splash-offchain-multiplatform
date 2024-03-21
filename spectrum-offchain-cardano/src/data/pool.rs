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
use cml_chain::utils::BigInt;

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
use spectrum_cardano_lib::plutus_data::{
    ConstrPlutusDataExtension, PlutusDataExtension,
};
use spectrum_cardano_lib::protocol_params::constant_tx_builder;
use spectrum_cardano_lib::transaction::TransactionOutputExtension;

use spectrum_cardano_lib::types::TryFromPData;
use spectrum_cardano_lib::{AssetClass, OutputRef, TaggedAmount, TaggedAssetClass, Token};
use spectrum_offchain::data::unique_entity::Predicted;
use spectrum_offchain::data::{Has, Stable, Tradable};
use spectrum_offchain::executor::RunOrderError::Fatal;

use spectrum_offchain::executor::{RunOrder, RunOrderError};
use spectrum_offchain::ledger::{IntoLedger, TryFromLedger};

use crate::data::balance_pool::BalancePool;
use crate::data::cfmm_pool::{CFMMPool, CFMMPoolRedeemer};

use crate::constants::{ORDER_EXECUTION_UNITS, POOL_EXECUTION_UNITS};
use crate::creds::OperatorRewardAddress;





use crate::data::order::{
    ClassicalOrderAction, ClassicalOrderRedeemer, PoolNft, Quote,
};

use crate::data::pair::PairId;
use crate::data::pool::AnyPool::{BalancedCFMM, PureCFMM};
use crate::data::pool::ApplyOrderError::{LowBatcherFeeErr, SlippageErr};



use crate::data::{OnChainOrderId, PoolVer};
use crate::deployment::ProtocolValidator::{BalanceFnPoolV1, ConstFnPoolV1, ConstFnPoolV2};
use crate::deployment::{DeployedValidator, RequiresValidator};



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
    PureCFMM(CFMMPool),
    BalancedCFMM(BalancePool),
}

pub struct AssetDeltas {
    pub asset_to_deduct_from: AssetClass,
    pub asset_to_add_to: AssetClass,
}

// impl TryFromLedger<BabbageTransactionOutput, OutputRef> for AnyPool {
//     fn try_from_ledger(repr: &BabbageTransactionOutput, ctx: OutputRef) -> Option<Self> {
//         if let Some(pool_ver) = PoolVer::try_from_address(repr.address()) {
//             return match pool_ver {
//                 PoolVer::V1 | PoolVer::V2 | PoolVer::FeeSwitch | PoolVer::FeeSwitchBiDirFee => {
//                     CFMMPool::try_from_ledger(repr, ctx).map(PureCFMM)
//                 }
//                 PoolVer::BalancePool => BalancePool::try_from_ledger(repr, ctx).map(BalancedCFMM),
//             };
//         }
//         None
//     }
// }

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
        // let x = self.asset_x.untag();
        // let y = self.asset_y.untag();
        // let [base, quote] = order_canonical(x, y);
        // let (base, quote) = match input {
        //     Side::Bid(input) => (
        //         self.output_amount(TaggedAssetClass::new(quote), TaggedAmount::new(input))
        //             .untag(),
        //         input,
        //     ),
        //     Side::Ask(input) => (
        //         input,
        //         self.output_amount(TaggedAssetClass::new(base), TaggedAmount::new(input))
        //             .untag(),
        //     ),
        // };
        // AbsolutePrice::new(quote, base)
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
        // let x = self.asset_x.untag();
        // let y = self.asset_y.untag();
        // let [base, quote] = order_canonical(x, y);
        // let output = match input {
        //     Side::Bid(input) => self
        //         .output_amount(TaggedAssetClass::new(quote), TaggedAmount::new(input))
        //         .untag(),
        //     Side::Ask(input) => self
        //         .output_amount(TaggedAssetClass::new(base), TaggedAmount::new(input))
        //         .untag(),
        // };
        // let (base_reserves, quote_reserves) = if x == base {
        //     (self.reserves_x.as_mut(), self.reserves_y.as_mut())
        // } else {
        //     (self.reserves_y.as_mut(), self.reserves_x.as_mut())
        // };
        // match input {
        //     Side::Bid(input) => {
        //         // A user bid means that they wish to buy the base asset for the quote asset, hence
        //         // pool reserves of base decreases while reserves of quote increase.
        //         *quote_reserves += input;
        //         *base_reserves -= output;
        //         (output, self)
        //     }
        //     Side::Ask(input) => {
        //         // User ask is the opposite; sell the base asset for the quote asset.
        //         *base_reserves += input;
        //         *quote_reserves -= output;
        //         (output, self)
        //     }
        // }
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
    C: Has<OutputRef>,
{
    fn try_from_ledger(repr: &BabbageTransactionOutput, ctx: C) -> Option<Self> {
        let cfmm_pool = CFMMPool::try_from_ledger(repr, ctx.get()).map(PureCFMM);
        let balance_pool = BalancePool::try_from_ledger(repr, ctx.get()).map(BalancedCFMM);
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

pub(crate) fn unsafe_update_datum(pool: &AnyPool, prev_datum: Option<DatumOption>) -> Option<DatumOption> {
    match prev_datum {
        Some(DatumOption::Datum {
            datum,
            len_encoding,
            tag_encoding,
            datum_tag_encoding,
            datum_bytes_encoding,
        }) => match pool {
            PureCFMM(pure_cfmm) => match pure_cfmm.ver {
                PoolVer::V1 | PoolVer::V2 => Some(DatumOption::Datum {
                    datum,
                    len_encoding,
                    tag_encoding,
                    datum_tag_encoding,
                    datum_bytes_encoding,
                }),
                PoolVer::FeeSwitch | PoolVer::FeeSwitchBiDirFee => {
                    let new_treasury_x = Integer(BigInt::from(pure_cfmm.treasury_x.untag()));
                    let new_treasury_y = Integer(BigInt::from(pure_cfmm.treasury_y.untag()));

                    let mut cpd = datum.into_constr_pd()?;

                    cpd.update_field_unsafe(6, new_treasury_x);
                    cpd.update_field_unsafe(7, new_treasury_y);

                    Some(DatumOption::Datum {
                        datum: PlutusData::ConstrPlutusData(cpd),
                        len_encoding,
                        tag_encoding,
                        datum_tag_encoding,
                        datum_bytes_encoding,
                    })
                }
                PoolVer::BalancePool => unreachable!(),
            },
            BalancedCFMM(pool) => {
                let new_treasury_x = Integer(BigInt::from(pool.treasury_x.untag()));
                let new_treasury_y = Integer(BigInt::from(pool.treasury_y.untag()));

                let mut cpd = datum.into_constr_pd()?;

                cpd.update_field_unsafe(9, new_treasury_x);
                cpd.update_field_unsafe(10, new_treasury_y);

                Some(DatumOption::Datum {
                    datum: PlutusData::ConstrPlutusData(cpd),
                    len_encoding,
                    tag_encoding,
                    datum_tag_encoding,
                    datum_bytes_encoding,
                })
            }
        },
        _ => panic!("Expected inline datum"),
    }
}

fn process_cml_step<Order>(
    step: Result<(), TxBuilderError>,
    order: Order,
) -> Result<(), RunOrderError<Order>> {
    match step {
        Ok(_) => Ok(()),
        Err(err) => {
            let error = format!("{:?}", err);
            Err(Fatal(error, order))
        }
    }
}

pub struct LegacyCFMMPoolConfig {
    pub pool_nft: TaggedAssetClass<PoolNft>,
    pub asset_x: TaggedAssetClass<Rx>,
    pub asset_y: TaggedAssetClass<Ry>,
    pub asset_lq: TaggedAssetClass<Lq>,
    pub lp_fee_num: u64,
    pub lq_lower_bound: TaggedAmount<Lq>,
}

impl TryFromPData for LegacyCFMMPoolConfig {
    fn try_from_pd(data: PlutusData) -> Option<Self> {
        let mut cpd = data.into_constr_pd()?;
        let pool_nft = TaggedAssetClass::try_from_pd(cpd.take_field(0)?)?;
        let asset_x = TaggedAssetClass::try_from_pd(cpd.take_field(1)?)?;
        let asset_y = TaggedAssetClass::try_from_pd(cpd.take_field(2)?)?;
        let asset_lq = TaggedAssetClass::try_from_pd(cpd.take_field(3)?)?;
        let lp_fee_num = cpd.take_field(4)?.into_u64()?;
        let lq_lower_bound = TaggedAmount::new(cpd.take_field(6).and_then(|pd| pd.into_u64()).unwrap_or(0));
        Some(Self {
            pool_nft,
            asset_x,
            asset_y,
            asset_lq,
            lp_fee_num,
            lq_lower_bound,
        })
    }
}

pub trait ApplyOrder<Order>: Sized {
    type Result;

    // return: new pool, order output
    fn apply_order(self, order: Order) -> Result<(Self, Self::Result), ApplyOrderError<Order>>;
}

pub struct RunAnyCFMMOrderOverPool<Pool>(pub Bundled<Pool, FinalizedTxOut>);

impl<'a, Order, Pool, Ctx> RunOrder<Bundled<Order, FinalizedTxOut>, Ctx, SignedTxBuilder>
    for RunAnyCFMMOrderOverPool<Pool>
where
    Pool:
        ApplyOrder<Order> + RequiresValidator<Ctx> + IntoLedger<TransactionOutput, ImmutablePoolUtxo> + Clone,
    <Pool as ApplyOrder<Order>>::Result: IntoLedger<TransactionOutput, Ctx>,
    Order: Has<OnChainOrderId> + RequiresValidator<Ctx> + Clone + Debug,
    Order: Into<CFMMPoolAction>,
    Ctx: Clone
        + Has<Collateral>
        + Has<OperatorRewardAddress>
        + Has<DeployedValidator<{ ConstFnPoolV1 as u8 }>>
        + Has<DeployedValidator<{ ConstFnPoolV2 as u8 }>>
        // balance pool redeem and deposit pipelines are the same as for const pool, except of pool script
        + Has<DeployedValidator<{ BalanceFnPoolV1 as u8 }>>,
{
    fn try_run(
        self,
        Bundled(order, FinalizedTxOut(order_utxo, order_ref)): Bundled<Order, FinalizedTxOut>,
        ctx: Ctx,
    ) -> Result<(SignedTxBuilder, Predicted<Self>), RunOrderError<Bundled<Order, FinalizedTxOut>>> {
        let RunAnyCFMMOrderOverPool(Bundled(pool, FinalizedTxOut(pool_utxo, pool_ref))) = self;
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
            .add_collateral(ctx.get_labeled::<Collateral>().into())
            .unwrap();

        tx_builder.add_reference_input(order_validator.reference_utxo);
        tx_builder.add_reference_input(pool_validator.reference_utxo);

        tx_builder.add_input(pool_in).unwrap();
        tx_builder.add_input(order_in).unwrap();

        tx_builder.set_exunits(
            RedeemerWitnessKey::new(RedeemerTag::Spend, pool_in_idx.clone().into()),
            POOL_EXECUTION_UNITS,
        );
        tx_builder.set_exunits(
            RedeemerWitnessKey::new(RedeemerTag::Spend, order_in_idx.clone().into()),
            ORDER_EXECUTION_UNITS,
        );

        tx_builder.add_output(SingleOutputBuilderResult::new(pool_out.clone()));

        // process_cml_step(
        //     tx_builder.add_output(SingleOutputBuilderResult::new(pool_out.clone())),
        //     order.clone(),
        // )?;

        tx_builder.add_output(SingleOutputBuilderResult::new(user_out.into_ledger(ctx.clone())));

        // process_cml_step(
        //     tx_builder.add_output(SingleOutputBuilderResult::new(user_out.into_ledger(ctx.clone()))),
        //     order,
        // )?;

        let tx = tx_builder
            .build(
                ChangeSelectionAlgo::Default,
                &ctx.get_labeled::<OperatorRewardAddress>().into(),
            )
            .unwrap();

        let tx_hash = hash_transaction_canonical(&tx.body());

        let next_pool_ref = OutputRef::new(tx_hash, 0);
        let predicted_pool = Predicted(RunAnyCFMMOrderOverPool(Bundled(
            next_pool,
            FinalizedTxOut(pool_out, next_pool_ref),
        )));

        Ok((tx, predicted_pool))
    }
}

/// Reference Script Output for [CFMMPool] tagged with pool version [Ver].
#[derive(Debug, Clone)]
pub struct CFMMPoolRefScriptOutput<const VER: u8>(pub TransactionUnspentOutput);

#[cfg(test)]
pub mod tests {
    use cml_crypto::TransactionHash;
    use rand::Rng;

    use bloom_offchain::execution_engine::liquidity_book::{pool::Pool, side::Side};
    use spectrum_cardano_lib::OutputRef;
    use spectrum_offchain::ledger::TryFromLedger;
    use test_utils::pool::gen_pool_transaction_output;

    use super::CFMMPool;

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

    fn gen_pool(ada_first: bool) -> CFMMPool {
        let (repr, _, _) = gen_pool_transaction_output(0, 101_000_000, 9_000_000, ada_first);
        let mut rng = rand::thread_rng();
        let mut bytes = [0u8; 32];
        rng.fill(&mut bytes[..]);
        let transaction_id = TransactionHash::from(bytes);
        let ctx = OutputRef::new(transaction_id, 0);
        CFMMPool::try_from_ledger(&repr, ctx).unwrap()
    }
}
