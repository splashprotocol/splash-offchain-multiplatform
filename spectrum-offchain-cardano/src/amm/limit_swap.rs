use cml_chain::builders::input_builder::SingleInputBuilder;
use cml_chain::builders::output_builder::SingleOutputBuilderResult;
use cml_chain::builders::redeemer_builder::RedeemerWitnessKey;
use cml_chain::builders::tx_builder::{ChangeSelectionAlgo, SignedTxBuilder};
use cml_chain::builders::witness_builder::{PartialPlutusWitness, PlutusScriptWitness};
use cml_chain::plutus::{ExUnits, PlutusData, RedeemerTag};
use cml_chain::transaction::TransactionOutput;
use cml_chain::Coin;
use cml_core::serialization::FromBytes;
use cml_crypto::Ed25519KeyHash;
use cml_multi_era::babbage::BabbageTransactionOutput;
use num_rational::Ratio;

use spectrum_cardano_lib::plutus_data::{
    ConstrPlutusDataExtension, DatumExtension, PlutusDataExtension, RequiresRedeemer,
};
use spectrum_cardano_lib::protocol_params::constant_tx_builder;
use spectrum_cardano_lib::transaction::TransactionOutputExtension;
use spectrum_cardano_lib::types::TryFromPData;
use spectrum_cardano_lib::value::ValueExtension;
use spectrum_cardano_lib::{AssetClass, OutputRef, TaggedAmount, TaggedAssetClass};
use spectrum_offchain::data::unique_entity::Predicted;
use spectrum_offchain::data::{Has, UniqueOrder};
use spectrum_offchain::executor::{RunOrder, RunOrderError};
use spectrum_offchain::ledger::{IntoLedger, TryFromLedger};

use crate::amm::execution_context::ExecutionContext;
use crate::amm::order::{Base, ClassicalOrder, ClassicalOrderAction, PoolNft, Quote};
use crate::amm::pool::{ApplySwap, CFMMPoolAction, ImmutablePoolUtxo};
use crate::amm::{ExecutorFeePerToken, OnChain, OnChainOrderId, PoolId, PoolStateVer, PoolVer};
use crate::constants::{MIN_SAFE_ADA_DEPOSIT, ORDER_APPLY_RAW_REDEEMER, ORDER_REFUND_RAW_REDEEMER};

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

impl TryFromLedger<BabbageTransactionOutput, OutputRef> for ClassicalOnChainLimitSwap {
    fn try_from_ledger(repr: BabbageTransactionOutput, ctx: OutputRef) -> Option<Self> {
        let value = repr.value().clone();
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
        if real_base_input < min_base || ada_deposit < MIN_SAFE_ADA_DEPOSIT {
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

impl<Swap, Pool> RunOrder<OnChain<Swap>, ExecutionContext, SignedTxBuilder> for OnChain<Pool>
where
    Pool: ApplySwap<Swap>
        + Has<PoolStateVer>
        + Has<PoolVer>
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
        ctx: ExecutionContext,
    ) -> Result<(SignedTxBuilder, Predicted<Self>), RunOrderError<OnChain<Swap>>> {
        let OnChain {
            value: pool,
            source: pool_out_in,
        } = self;
        let pool_ver = pool.get::<PoolVer>();
        let pool_ref = OutputRef::from(pool.get::<PoolStateVer>());
        let order_ref = OutputRef::from(order.get::<OnChainOrderId>());
        let (next_pool, swap_out) = match pool.apply_swap(order) {
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
        let mut tx_builder = constant_tx_builder();

        if pool_ver == PoolVer::v2() {
            tx_builder.add_reference_input(ctx.ref_scripts.pool_v2)
        } else {
            tx_builder.add_reference_input(ctx.ref_scripts.pool_v1);
        }

        tx_builder.add_collateral(ctx.collateral).unwrap();

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

        let tx = tx_builder
            .build(ChangeSelectionAlgo::Default, &ctx.operator_addr)
            .unwrap();

        Ok((tx, predicted_pool))
    }
}

#[cfg(test)]
mod tests {
    use cml_chain::address::EnterpriseAddress;
    use cml_chain::certs::StakeCredential;
    use cml_chain::genesis::network_info::NetworkInfo;
    use cml_chain::plutus::PlutusData;
    use cml_chain::Deserialize;
    use cml_crypto::{Ed25519KeyHash, TransactionHash};
    use cml_multi_era::babbage::BabbageTransactionOutput;

    use cardano_explorer::client::Explorer;
    use cardano_explorer::data::ExplorerConfig;
    use spectrum_cardano_lib::types::TryFromPData;
    use spectrum_cardano_lib::OutputRef;
    use spectrum_offchain::executor::RunOrder;
    use spectrum_offchain::ledger::TryFromLedger;

    use crate::amm::execution_context::ExecutionContext;
    use crate::amm::limit_swap::OnChainLimitSwapConfig;
    use crate::amm::order::ClassicalOnChainOrder;
    use crate::amm::pool::CFMMPool;
    use crate::amm::ref_scripts::RefScriptsOutputs;
    use crate::amm::OnChain;
    use crate::collateral_storage::CollateralStorage;
    use crate::config::RefScriptsConfig;

    #[test]
    fn parse_swap_datum_mainnet() {
        let pd = PlutusData::from_cbor_bytes(&*hex::decode(DATUM_SAMPLE).unwrap()).unwrap();
        let maybe_conf = OnChainLimitSwapConfig::try_from_pd(pd);
        assert!(maybe_conf.is_some())
    }

    const DATUM_SAMPLE: &str =
        "d8799fd8799f581c95a427e384527065f2f8946f5e86320d0117839a5e98ea2c0b55fb004448554e54ffd8799f\
        4040ffd8799f581ce08fbaa73db55294b3b31f2a365be5c4b38211a47880f0ef6b17a1604c48554e545f4144415\
        f4e4654ff1903e51b00148f1c351223aa1b8ac7230489e80000581c022835b77a25d6bf00f8cbf7e4744e0065ec\
        77383500221ed4f32514d8799f581c3cc6ea3784eecc03bc736d90e368abb40f873c48d1fc74133afae5a5ff1b0\
        0000003882d614c1a9a800dc5ff";

    #[tokio::test]
    async fn run_valid_swap_against_pool() {
        let swap_ref = OutputRef::from((TransactionHash::from([0u8; 32]), 0));
        let pool_ref = OutputRef::from((TransactionHash::from([1u8; 32]), 0));
        let swap_box =
            BabbageTransactionOutput::from_cbor_bytes(&*hex::decode(SWAP_SAMPLE).unwrap()).unwrap();
        let pool_box =
            BabbageTransactionOutput::from_cbor_bytes(&*hex::decode(POOL_SAMPLE).unwrap()).unwrap();
        let swap = ClassicalOnChainOrder::try_from_ledger(swap_box, swap_ref).unwrap();
        let pool = <OnChain<CFMMPool>>::try_from_ledger(pool_box, pool_ref).unwrap();

        let operator_pkh =
            Ed25519KeyHash::from_hex("fbbbd406cb78494b84fb7497e9abe0f942fee10b85b8c01c5b1bb500").unwrap();
        let operator_addr = EnterpriseAddress::new(
            NetworkInfo::mainnet().network_id(),
            StakeCredential::new_pub_key(operator_pkh),
        )
        .to_address();
        let collateral_storage = CollateralStorage::new(operator_pkh.to_hex());

        let explorer = Explorer::new(ExplorerConfig {
            url: "https://explorer.spectrum.fi",
        });
        let ref_scripts_conf = RefScriptsConfig {
            pool_v1_ref: "31a497ef6b0033e66862546aa2928a1987f8db3b8f93c59febbe0f47b14a83c6#0".to_string(),
            pool_v2_ref: "c8c93656e8bce07fabe2f42d703060b7c71bfa2e48a2956820d1bd81cc936faa#0".to_string(),
            swap_ref: "fc9e99fd12a13a137725da61e57a410e36747d513b965993d92c32c67df9259a#2".to_string(),
            deposit_ref: "fc9e99fd12a13a137725da61e57a410e36747d513b965993d92c32c67df9259a#0".to_string(),
            redeem_ref: "fc9e99fd12a13a137725da61e57a410e36747d513b965993d92c32c67df9259a#1".to_string(),
        };
        let ref_scripts = RefScriptsOutputs::new(ref_scripts_conf, explorer)
            .await
            .expect("Ref scripts initialization failed");
        let collateral = collateral_storage
            .get_collateral(explorer)
            .await
            .expect("Couldn't retrieve collateral");

        let ctx = ExecutionContext::new(operator_addr, ref_scripts, collateral);

        let result = pool.try_run(swap, ctx);

        assert!(result.is_ok())
    }

    const SWAP_SAMPLE: &str = "a300581d712618e94cdb06792f05ae9b1ec78b0231f4b7f4215b1b4cf52e6342de01821a003d0900a1581c279c909f348e533da5808898f87f9a14bb2c3dfbbacccd631d927a3fa144534e454b1a0008785c028201d81858bfd8799fd8799f581c279c909f348e533da5808898f87f9a14bb2c3dfbbacccd631d927a3f44534e454bffd8799f4040ffd8799f581c4a27465112a39464e6dd5ee470c552ebb3cb42925d5ec040149679084c534e454b5f4144415f4e4654ff1903e51b000e8572e9e6ad3b1b0de0b6b3a7640000581cf80732ec4932b37d388b4234f20435ab4b6c5975456537722c84c036d8799f581c927fc9f34299075355c9c309dfafefb5c00f20f4651a5780e241322bff1a0008785c1a15dfb8e4ff";
    const POOL_SAMPLE: &str = "a300583931e628bfd68c07a7a38fcd7d8df650812a9dfdbee54b1ed4c25c87ffbfb2f6abf60ccde92eae1a2f4fdf65f2eaf6208d872c6f0e597cc10b0701821b0000000b07810d11a3581c279c909f348e533da5808898f87f9a14bb2c3dfbbacccd631d927a3fa144534e454b1a043526c2581c4a27465112a39464e6dd5ee470c552ebb3cb42925d5ec04014967908a14c534e454b5f4144415f4e465401581c7bddf2c27f257eeeef3e892758b479e09c89a73642499797f2a97f3ca14b534e454b5f4144415f4c511b7fffffff939f34ff028201d81858bad8799fd8799f581c4a27465112a39464e6dd5ee470c552ebb3cb42925d5ec040149679084c534e454b5f4144415f4e4654ffd8799f4040ffd8799f581c279c909f348e533da5808898f87f9a14bb2c3dfbbacccd631d927a3f44534e454bffd8799f581c7bddf2c27f257eeeef3e892758b479e09c89a73642499797f2a97f3c4b534e454b5f4144415f4c51ff1903e59f581c856e34eac199979f7c04d4b500c6e91748dec14d92a28b3c1bf75882ff1b00000004a817c800ff";
}
