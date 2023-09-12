use cml_chain::builders::input_builder::SingleInputBuilder;
use cml_chain::builders::output_builder::SingleOutputBuilderResult;
use cml_chain::builders::tx_builder::{
    ChangeSelectionAlgo, SignedTxBuilder, TransactionBuilder, TransactionBuilderConfig,
};
use cml_chain::builders::witness_builder::{PartialPlutusWitness, PlutusScriptWitness};
use cml_chain::Coin;
use cml_chain::plutus::PlutusData;
use cml_chain::transaction::TransactionOutput;
use cml_crypto::Ed25519KeyHash;
use num_rational::Ratio;

use spectrum_cardano_lib::{AssetClass, OutputRef, TaggedAmount, TaggedAssetClass};
use spectrum_cardano_lib::plutus_data::{
    ConstrPlutusDataExtension, DatumExtension, PlutusDataExtension, RequiresRedeemer,
};
use spectrum_cardano_lib::transaction::TransactionOutputExtension;
use spectrum_cardano_lib::types::TryFromPData;
use spectrum_cardano_lib::value::ValueExtension;
use spectrum_offchain::data::{Has, UniqueOrder};
use spectrum_offchain::data::unique_entity::Predicted;
use spectrum_offchain::executor::{RunOrder, RunOrderError};
use spectrum_offchain::ledger::{IntoLedger, TryFromLedger};

use crate::data::{ExecutorFeePerToken, OnChain, OnChainOrderId, PoolId};
use crate::data::order::{Base, ClassicalOrder, ClassicalOrderAction, PoolNft, Quote};
use crate::data::pool::{ApplySwap, CFMMPoolAction};

#[derive(Debug, Clone)]
pub struct LimitSwap {
    pub base_asset: TaggedAssetClass<Base>,
    pub base_amount: TaggedAmount<Base>,
    pub quote_asset: TaggedAssetClass<Quote>,
    pub ada_deposit: Coin,
    pub min_expected_quote_amount: TaggedAmount<Quote>,
    pub fee: ExecutorFeePerToken,
    pub redeemer_pkh: Ed25519KeyHash,
}

pub type ClassicalOnChainLimitSwap = ClassicalOrder<OnChainOrderId, LimitSwap>;

impl UniqueOrder for ClassicalOnChainLimitSwap {
    type TOrderId = OnChainOrderId;
    fn get_self_ref(&self) -> Self::TOrderId {
        self.id
    }
}

impl TryFromLedger<TransactionOutput, OutputRef> for ClassicalOnChainLimitSwap {
    fn try_from_ledger(repr: TransactionOutput, ctx: OutputRef) -> Option<Self> {
        let value = repr.amount().clone();
        let conf = OnChainLimitSwapConfig::try_from_pd(repr.into_datum()?.into_pd()?)?;
        let real_base_input = value.amount_of(conf.base.untag()).unwrap_or(0);
        let (min_base, ada_deposit) = if conf.base.is_native() {
            let min = conf.base_amount.untag()
                + ((conf.min_quote_amount.untag() as u128) * (conf.ex_fee_per_token_num as u128)
                    / (conf.ex_fee_per_token_denom as u128)) as u64;
            let ada = real_base_input - conf.base_amount.untag();
            (min, ada)
        } else {
            (conf.base_amount.untag(), value.coin)
        };
        if real_base_input < min_base {
            return None;
        }
        let swap = LimitSwap {
            base_asset: conf.base,
            base_amount: conf.base_amount,
            quote_asset: conf.quote,
            min_expected_quote_amount: conf.min_quote_amount,
            ada_deposit,
            fee: ExecutorFeePerToken::new(
                Ratio::new(conf.ex_fee_per_token_num, conf.ex_fee_per_token_denom),
                AssetClass::Native,
            ),
            redeemer_pkh: conf.redeemer_pkh,
        };
        Some(ClassicalOrder {
            id: OnChainOrderId::from(ctx),
            pool_id: PoolId::try_from(conf.pool_nft).ok()?,
            order: swap,
        })
    }
}

pub struct OnChainLimitSwapConfig {
    pub base: TaggedAssetClass<Base>,
    pub base_amount: TaggedAmount<Base>,
    pub quote: TaggedAssetClass<Quote>,
    pub min_quote_amount: TaggedAmount<Quote>,
    pub pool_nft: TaggedAssetClass<PoolNft>,
    pub ex_fee_per_token_num: u64,
    pub ex_fee_per_token_denom: u64,
    pub redeemer_pkh: Ed25519KeyHash,
}

impl TryFromPData for OnChainLimitSwapConfig {
    fn try_from_pd(data: PlutusData) -> Option<Self> {
        let mut cpd = data.into_constr_pd()?;
        Some(OnChainLimitSwapConfig {
            base: TaggedAssetClass::try_from_pd(cpd.take_field(0)?)?,
            base_amount: TaggedAmount::try_from_pd(cpd.take_field(8)?)?,
            quote: TaggedAssetClass::try_from_pd(cpd.take_field(1)?)?,
            min_quote_amount: TaggedAmount::try_from_pd(cpd.take_field(9)?)?,
            pool_nft: TaggedAssetClass::try_from_pd(cpd.take_field(2)?)?,
            ex_fee_per_token_num: cpd.take_field(4)?.into_u64()?,
            ex_fee_per_token_denom: cpd.take_field(5)?.into_u64()?,
            redeemer_pkh: Ed25519KeyHash::from(<[u8; 28]>::try_from(cpd.take_field(6)?.into_bytes()?).ok()?),
        })
    }
}

impl<Swap, Pool> RunOrder<OnChain<Swap>, TransactionBuilderConfig, SignedTxBuilder> for OnChain<Pool>
where
    Pool: ApplySwap<Swap>
        + Has<OutputRef>
        + RequiresRedeemer<CFMMPoolAction>
        + IntoLedger<TransactionOutput>
        + Clone,
    Swap: Has<OutputRef> + RequiresRedeemer<ClassicalOrderAction>,
{
    fn try_run(
        self,
        OnChain {
            value: order,
            source: order_out_in,
        }: OnChain<Swap>,
        ctx: TransactionBuilderConfig,
    ) -> Result<(SignedTxBuilder, Predicted<Self>), RunOrderError<OnChain<Swap>>> {
        let OnChain {
            value: pool,
            source: pool_out_in,
        } = self;
        let pool_ref = pool.get::<OutputRef>();
        let order_ref = order.get::<OutputRef>();
        let (next_pool, swap_out, batcher_profit) = match pool.apply_swap(order) {
            Ok(res) => res,
            Err(slippage) => {
                return Err(slippage
                    .map(|value| OnChain {
                        value,
                        source: order_out_in,
                    })
                    .into())
            }
        };
        let pool_redeemer = Pool::redeemer(CFMMPoolAction::Swap);
        let pool_script = PartialPlutusWitness::new(
            PlutusScriptWitness::Ref(pool_out_in.script_hash().unwrap()),
            pool_redeemer,
        );
        let pool_datum = pool_out_in.datum().unwrap().into_pd().unwrap();
        let pool_in = SingleInputBuilder::new(pool_ref.into(), pool_out_in)
            .plutus_script(pool_script, Vec::new(), pool_datum)
            .unwrap();
        let order_redeemer = Swap::redeemer(ClassicalOrderAction::Apply);
        let order_script = PartialPlutusWitness::new(
            PlutusScriptWitness::Ref(order_out_in.script_hash().unwrap()),
            order_redeemer,
        );
        let order_datum = order_out_in.datum().unwrap().into_pd().unwrap();
        let order_in = SingleInputBuilder::new(order_ref.into(), order_out_in)
            .plutus_script(order_script, Vec::new(), order_datum)
            .unwrap();
        let pool_out = next_pool.clone().into_ledger();
        let predicted_pool = OnChain {
            value: next_pool,
            source: pool_out.clone(),
        };
        let user_out = swap_out.into_ledger();
        let batcher_out = batcher_profit.into_ledger();
        let batcher_addr = batcher_out.address().clone();
        let mut tx_builder = TransactionBuilder::new(ctx);
        tx_builder.add_input(pool_in);
        tx_builder.add_input(order_in);
        tx_builder.add_output(SingleOutputBuilderResult::new(pool_out));
        tx_builder.add_output(SingleOutputBuilderResult::new(user_out));
        tx_builder.add_output(SingleOutputBuilderResult::new(batcher_out));
        let tx = tx_builder
            .build(ChangeSelectionAlgo::Default, &batcher_addr)
            .unwrap();
        Ok((tx, Predicted(predicted_pool)))
    }
}
