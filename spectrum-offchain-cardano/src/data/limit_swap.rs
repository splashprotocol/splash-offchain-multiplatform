use cml_chain::address::Address;
use cml_chain::builders::input_builder::SingleInputBuilder;
use cml_chain::builders::output_builder::SingleOutputBuilderResult;
use cml_chain::builders::redeemer_builder::RedeemerWitnessKey;
use cml_chain::builders::tx_builder::{ChangeSelectionAlgo, SignedTxBuilder};
use cml_chain::builders::witness_builder::{PartialPlutusWitness, PlutusScriptWitness};
use cml_chain::crypto::hash::hash_transaction;
use cml_chain::crypto::utils::make_vkey_witness;
use cml_chain::plutus::{ExUnits, PlutusData, RedeemerTag};
use cml_chain::transaction::TransactionOutput;
use cml_chain::Coin;
use cml_crypto::Ed25519KeyHash;

use cml_core::serialization::FromBytes;

use num_rational::Ratio;

use crate::cardano::protocol_params::constant_tx_builder;
use crate::constants::{ORDER_APPLY_RAW_REDEEMER, ORDER_REFUND_RAW_REDEEMER};
use spectrum_cardano_lib::plutus_data::{
    ConstrPlutusDataExtension, DatumExtension, PlutusDataExtension, RequiresRedeemer,
};
use spectrum_cardano_lib::transaction::TransactionOutputExtension;
use spectrum_cardano_lib::types::TryFromPData;
use spectrum_cardano_lib::value::ValueExtension;
use spectrum_cardano_lib::{AssetClass, OutputRef, TaggedAmount, TaggedAssetClass};
use spectrum_offchain::data::unique_entity::Predicted;
use spectrum_offchain::data::{Has, UniqueOrder};
use spectrum_offchain::executor::{RunOrder, RunOrderError};
use spectrum_offchain::ledger::{IntoLedger, TryFromLedger};

use crate::data::order::{Base, ClassicalOrder, ClassicalOrderAction, PoolNft, Quote};
use crate::data::order_execution_context::OrderExecutionContext;
use crate::data::pool::{ApplySwap, CFMMPoolAction, ImmutablePoolUtxo};
use crate::data::{ExecutorFeePerToken, OnChain, OnChainOrderId, PoolId, PoolStateVer};

#[derive(Debug, Clone)]
pub struct LimitSwap {
    pub base_asset: TaggedAssetClass<Base>,
    pub base_amount: TaggedAmount<Base>,
    pub quote_asset: TaggedAssetClass<Quote>,
    pub ada_deposit: Coin,
    pub min_expected_quote_amount: TaggedAmount<Quote>,
    pub fee: ExecutorFeePerToken,
    pub redeemer_pkh: Ed25519KeyHash,
    pub redeemer_stake_pkh: Option<Ed25519KeyHash>,
}

pub type ClassicalOnChainLimitSwap = ClassicalOrder<OnChainOrderId, LimitSwap>;

impl RequiresRedeemer<ClassicalOrderAction> for ClassicalOnChainLimitSwap {
    fn redeemer(action: ClassicalOrderAction) -> PlutusData {
        match action {
            ClassicalOrderAction::Apply => {
                PlutusData::from_bytes(hex::decode(ORDER_APPLY_RAW_REDEEMER).unwrap()).unwrap()
            }
            ClassicalOrderAction::Refund => {
                PlutusData::from_bytes(hex::decode(ORDER_REFUND_RAW_REDEEMER).unwrap()).unwrap()
            }
        }
    }
}

impl UniqueOrder for ClassicalOnChainLimitSwap {
    type TOrderId = OnChainOrderId;
    fn get_self_ref(&self) -> Self::TOrderId {
        self.id
    }
}

impl TryFromLedger<TransactionOutput, OutputRef> for ClassicalOnChainLimitSwap {
    fn try_from_ledger(repr: TransactionOutput, ctx: OutputRef) -> Option<Self> {
        let value = repr.amount().clone();
        let conf = OnChainLimitSwapConfig::try_from_pd(repr.clone().into_datum()?.into_pd()?)?;
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
            redeemer_stake_pkh: conf.redeemer_stake_pkh,
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
    pub redeemer_stake_pkh: Option<Ed25519KeyHash>,
}

impl TryFromPData for OnChainLimitSwapConfig {
    fn try_from_pd(data: PlutusData) -> Option<Self> {
        let mut cpd = data.into_constr_pd()?;
        let stake_pkh: Option<Ed25519KeyHash> = cpd
            .take_field(7)
            .and_then(|pd| pd.into_bytes())
            .and_then(|bytes| <[u8; 28]>::try_from(bytes).ok())
            .map(|bytes| Ed25519KeyHash::from(bytes));

        Some(OnChainLimitSwapConfig {
            base: TaggedAssetClass::try_from_pd(cpd.take_field(0)?)?,
            base_amount: TaggedAmount::try_from_pd(cpd.take_field(8)?)?,
            quote: TaggedAssetClass::try_from_pd(cpd.take_field(1)?)?,
            min_quote_amount: TaggedAmount::try_from_pd(cpd.take_field(9)?)?,
            pool_nft: TaggedAssetClass::try_from_pd(cpd.take_field(2)?)?,
            ex_fee_per_token_num: cpd.take_field(4)?.into_u64()?,
            ex_fee_per_token_denom: cpd.take_field(5)?.into_u64()?,
            redeemer_pkh: Ed25519KeyHash::from(<[u8; 28]>::try_from(cpd.take_field(6)?.into_bytes()?).ok()?),
            redeemer_stake_pkh: stake_pkh,
        })
    }
}

impl<'a, Swap, Pool> RunOrder<OnChain<Swap>, OrderExecutionContext<'a>, SignedTxBuilder> for OnChain<Pool>
where
    Pool: ApplySwap<Swap>
        + Has<PoolStateVer>
        + RequiresRedeemer<CFMMPoolAction>
        + IntoLedger<TransactionOutput, ImmutablePoolUtxo>
        + Clone,
    Swap: Has<OnChainOrderId> + RequiresRedeemer<ClassicalOrderAction>,
{
    fn try_run(
        self,
        OnChain {
            value: order,
            source: order_out_in,
        }: OnChain<Swap>,
        ctx: OrderExecutionContext,
    ) -> Result<(SignedTxBuilder, Predicted<Self>), RunOrderError<OnChain<Swap>>> {
        let OnChain {
            value: pool,
            source: pool_out_in,
        } = self;
        let pool_ref = OutputRef::from(pool.get::<PoolStateVer>());
        let order_ref = OutputRef::from(order.get::<OnChainOrderId>());
        let (next_pool, swap_out, batcher_profit) = match pool.apply_swap(order, ctx.batcher_pkh) {
            Ok(res) => res,
            Err(slippage) => {
                return Err(slippage
                    .map(|value| OnChain {
                        value,
                        source: order_out_in,
                    })
                    .into());
            }
        };
        let pool_redeemer = Pool::redeemer(CFMMPoolAction::Swap);
        let pool_script = PartialPlutusWitness::new(
            PlutusScriptWitness::Ref(pool_out_in.script_hash().unwrap()),
            pool_redeemer,
        );
        let pool_datum = pool_out_in.datum().unwrap().into_pd().unwrap();
        let immut_pool = ImmutablePoolUtxo::from(&pool_out_in);
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
        let pool_out = next_pool.clone().into_ledger(immut_pool);
        let predicted_pool = Predicted(OnChain {
            value: next_pool,
            source: pool_out.clone(),
        });
        let user_out = swap_out.into_ledger(());
        let batcher_out = batcher_profit.into_ledger(());
        let batcher_addr = batcher_out.address().clone();
        let mut tx_builder = constant_tx_builder();

        //todo: add pools version and remove if-else. PoolStateVer?
        let pool_parsed_address =
            Address::to_bech32(pool_in.utxo_info.address(), None).unwrap_or(String::from("unknown"));

        if (pool_parsed_address == "addr1x94ec3t25egvhqy2n265xfhq882jxhkknurfe9ny4rl9k6dj764lvrxdayh2ux30fl0ktuh27csgmpevdu89jlxppvrst84slu") {
            tx_builder
                .add_reference_input(ctx.ref_scripts.pool_v2)
        } else if (pool_parsed_address == "addr1x8nz307k3sr60gu0e47cmajssy4fmld7u493a4xztjrll0aj764lvrxdayh2ux30fl0ktuh27csgmpevdu89jlxppvrswgxsta") {
            tx_builder
                .add_reference_input(ctx.ref_scripts.pool_v1);
        }

        tx_builder.add_collateral(ctx.collateral);

        tx_builder.add_reference_input(ctx.ref_scripts.swap);

        tx_builder.add_input(pool_in.clone()).unwrap();
        tx_builder.add_input(order_in).unwrap();

        tx_builder.set_exunits(
            RedeemerWitnessKey::new(RedeemerTag::Spend, 0),
            ExUnits::new(530000, 165000000),
        );

        tx_builder.set_exunits(
            RedeemerWitnessKey::new(RedeemerTag::Spend, 1),
            ExUnits::new(270000, 140000000),
        );

        tx_builder
            .add_output(SingleOutputBuilderResult::new(pool_out))
            .unwrap();
        tx_builder
            .add_output(SingleOutputBuilderResult::new(user_out))
            .unwrap();

        let mut tx = tx_builder
            .build(ChangeSelectionAlgo::Default, &batcher_addr)
            .unwrap();

        let body = tx.body();

        let batcher_signature = make_vkey_witness(&hash_transaction(&body), ctx.batcher_private);

        tx.add_vkey(batcher_signature);

        Ok((tx, predicted_pool))
    }
}

#[cfg(test)]
mod tests {
    use cml_chain::plutus::PlutusData;
    use cml_chain::Deserialize;

    use spectrum_cardano_lib::types::TryFromPData;

    use crate::data::limit_swap::OnChainLimitSwapConfig;

    #[test]
    fn parse_swap_datum_mainnet() {
        let pd = PlutusData::from_cbor_bytes(&*hex::decode(DATUM).unwrap()).unwrap();
        let maybe_conf = OnChainLimitSwapConfig::try_from_pd(pd);
        assert!(maybe_conf.is_some())
    }

    const DATUM: &str =
        "d8799fd8799f581c95a427e384527065f2f8946f5e86320d0117839a5e98ea2c0b55fb004448554e54ffd8799f\
        4040ffd8799f581ce08fbaa73db55294b3b31f2a365be5c4b38211a47880f0ef6b17a1604c48554e545f4144415\
        f4e4654ff1903e51b00148f1c351223aa1b8ac7230489e80000581c022835b77a25d6bf00f8cbf7e4744e0065ec\
        77383500221ed4f32514d8799f581c3cc6ea3784eecc03bc736d90e368abb40f873c48d1fc74133afae5a5ff1b0\
        0000003882d614c1a9a800dc5ff";
}
