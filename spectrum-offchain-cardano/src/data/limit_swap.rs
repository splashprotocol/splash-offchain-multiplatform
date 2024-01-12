use cml_chain::plutus::PlutusData;
use cml_chain::Coin;
use cml_core::serialization::FromBytes;
use cml_crypto::Ed25519KeyHash;
use cml_multi_era::babbage::BabbageTransactionOutput;
use num_rational::Ratio;

use spectrum_cardano_lib::plutus_data::{
    ConstrPlutusDataExtension, DatumExtension, PlutusDataExtension, RequiresRedeemer,
};
use spectrum_cardano_lib::transaction::TransactionOutputExtension;
use spectrum_cardano_lib::types::TryFromPData;
use spectrum_cardano_lib::value::ValueExtension;
use spectrum_cardano_lib::{AssetClass, OutputRef, TaggedAmount, TaggedAssetClass};
use spectrum_offchain::data::order::UniqueOrder;
use spectrum_offchain::ledger::TryFromLedger;

use crate::constants::{MIN_SAFE_ADA_VALUE, ORDER_APPLY_RAW_REDEEMER, ORDER_REFUND_RAW_REDEEMER};
use crate::data::order::{Base, ClassicalOrder, ClassicalOrderAction, PoolNft, Quote};
use crate::data::pool::CFMMPoolAction;
use crate::data::pool::CFMMPoolAction::Swap;
use crate::data::{ExecutorFeePerToken, OnChainOrderId, PoolId};

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

impl Into<CFMMPoolAction> for ClassicalOnChainLimitSwap {
    fn into(self) -> CFMMPoolAction {
        Swap
    }
}

impl UniqueOrder for ClassicalOnChainLimitSwap {
    type TOrderId = OnChainOrderId;
    fn get_self_ref(&self) -> Self::TOrderId {
        self.id
    }
}

impl TryFromLedger<BabbageTransactionOutput, OutputRef> for ClassicalOnChainLimitSwap {
    fn try_from_ledger(repr: &BabbageTransactionOutput, ctx: OutputRef) -> Option<Self> {
        let value = repr.value().clone();
        let conf = OnChainLimitSwapConfig::try_from_pd(repr.datum()?.into_pd()?)?;
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
        if real_base_input < min_base || ada_deposit < MIN_SAFE_ADA_VALUE {
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
    pub ex_fee_per_token_num: u128,
    pub ex_fee_per_token_denom: u128,
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
            ex_fee_per_token_num: cpd.take_field(4)?.into_u128()?,
            ex_fee_per_token_denom: cpd.take_field(5)?.into_u128()?,
            redeemer_pkh: Ed25519KeyHash::from(<[u8; 28]>::try_from(cpd.take_field(6)?.into_bytes()?).ok()?),
            redeemer_stake_pkh: stake_pkh,
        })
    }
}

#[cfg(test)]
mod tests {
    use cml_chain::address::EnterpriseAddress;
    use cml_chain::certs::StakeCredential;
    use cml_chain::genesis::network_info::NetworkInfo;
    use cml_chain::plutus::PlutusData;
    use cml_chain::transaction::TransactionOutput;
    use cml_chain::{Deserialize, Value};
    use cml_crypto::{Bip32PrivateKey, TransactionHash};
    use cml_multi_era::babbage::BabbageTransactionOutput;

    use cardano_explorer::client::Explorer;
    use cardano_explorer::data::ExplorerConfig;
    use spectrum_cardano_lib::types::TryFromPData;
    use spectrum_cardano_lib::OutputRef;
    use spectrum_offchain::executor::RunOrder;
    use spectrum_offchain::ledger::TryFromLedger;

    use crate::collaterals::tests::MockBasedRequestor;
    use crate::collaterals::Collaterals;
    use crate::creds::operator_creds;
    use crate::data::execution_context::ExecutionContext;
    use crate::data::limit_swap::OnChainLimitSwapConfig;
    use crate::data::order::ClassicalOnChainOrder;
    use crate::data::pool::CFMMPool;
    use crate::data::ref_scripts::ReferenceOutputs;
    use crate::data::OnChain;
    use crate::ref_scripts::ReferenceSources;

    #[test]
    fn parse_swap_datum_mainnet() {
        let pd = PlutusData::from_cbor_bytes(&*hex::decode(DATUM_SAMPLE).unwrap()).unwrap();
        let maybe_conf = OnChainLimitSwapConfig::try_from_pd(pd);
        assert!(maybe_conf.is_some())
    }

    const DATUM_SAMPLE: &str =
        "d8799fd8799f581c5d16cc1a177b5d9ba9cfa9793b07e60f1fb70fea1f8aef064415d11443494147ffd8799f4040ffd8799f581cb99258\
        2b95a3ee20cb4025699808c83caaefa7bae9387b72ba2c57c34b4941475f4144415f4e4654ff1903e51b008269b69644065ec249056bc75\
        e2d63100000581cf7cfe1832d19b34789d6c3042eeb674907a292f63ba0d4cdbb140689d8799f581c196d9fb5061b3a658eae958c155405\
        014b1a01c60744c57241e0a374ff1b000000029cf3700f1af39010b7ff";

    #[tokio::test]
    async fn run_valid_swap_against_pool() {
        let swap_ref = OutputRef::from((TransactionHash::from([0u8; 32]), 0));
        let pool_ref = OutputRef::from((TransactionHash::from([1u8; 32]), 0));
        let swap_box =
            BabbageTransactionOutput::from_cbor_bytes(&*hex::decode(SWAP_SAMPLE).unwrap()).unwrap();
        let pool_box =
            BabbageTransactionOutput::from_cbor_bytes(&*hex::decode(POOL_SAMPLE).unwrap()).unwrap();
        let swap = ClassicalOnChainOrder::try_from_ledger(&swap_box, swap_ref).unwrap();
        let pool = <OnChain<CFMMPool>>::try_from_ledger(&pool_box, pool_ref).unwrap();

        let private_key_bech32 = Bip32PrivateKey::generate_ed25519_bip32().to_bech32();

        let (_, operator_pkh, operator_addr) =
            operator_creds(private_key_bech32.as_str(), NetworkInfo::mainnet());

        let test_address = EnterpriseAddress::new(
            NetworkInfo::mainnet().network_id(),
            StakeCredential::new_pub_key(operator_pkh),
        )
        .to_address();

        let collateral_output: TransactionOutput = TransactionOutput::new(
            test_address,
            Value::from(10000000),
            None,
            None, // todo: explorer doesn't support script ref. Change to correct after explorer update
        );

        let mock_requestor = MockBasedRequestor::new(collateral_output);

        let explorer = Explorer::new(ExplorerConfig {
            url: "https://explorer.spectrum.fi",
        });
        let ref_scripts_conf = ReferenceSources {
            pool_v1_script: "31a497ef6b0033e66862546aa2928a1987f8db3b8f93c59febbe0f47b14a83c6#0"
                .try_into()
                .unwrap(),
            pool_v2_script: "c8c93656e8bce07fabe2f42d703060b7c71bfa2e48a2956820d1bd81cc936faa#0"
                .try_into()
                .unwrap(),
            swap_script: "fc9e99fd12a13a137725da61e57a410e36747d513b965993d92c32c67df9259a#2"
                .try_into()
                .unwrap(),
            deposit_script: "fc9e99fd12a13a137725da61e57a410e36747d513b965993d92c32c67df9259a#0"
                .try_into()
                .unwrap(),
            redeem_script: "fc9e99fd12a13a137725da61e57a410e36747d513b965993d92c32c67df9259a#1"
                .try_into()
                .unwrap(),
        };
        let ref_scripts = ReferenceOutputs::pull(ref_scripts_conf, explorer)
            .await
            .expect("Ref scripts initialization failed");
        let collateral = mock_requestor
            .get_collateral()
            .await
            .expect("Couldn't retrieve collateral");

        let ctx = ExecutionContext::new(operator_addr, ref_scripts, collateral.into());

        let result = pool.try_run(swap, ctx);

        assert!(result.is_ok())
    }

    const SWAP_SAMPLE: &str = "a300581d712618e94cdb06792f05ae9b1ec78b0231f4b7f4215b1b4cf52e6342de01821a003d0900a1581c279c909f348e533da5808898f87f9a14bb2c3dfbbacccd631d927a3fa144534e454b1a0008785c028201d81858bfd8799fd8799f581c279c909f348e533da5808898f87f9a14bb2c3dfbbacccd631d927a3f44534e454bffd8799f4040ffd8799f581c4a27465112a39464e6dd5ee470c552ebb3cb42925d5ec040149679084c534e454b5f4144415f4e4654ff1903e51b000e8572e9e6ad3b1b0de0b6b3a7640000581cf80732ec4932b37d388b4234f20435ab4b6c5975456537722c84c036d8799f581c927fc9f34299075355c9c309dfafefb5c00f20f4651a5780e241322bff1a0008785c1a15dfb8e4ff";
    const POOL_SAMPLE: &str = "a300583931e628bfd68c07a7a38fcd7d8df650812a9dfdbee54b1ed4c25c87ffbfb2f6abf60ccde92eae1a2f4fdf65f2eaf6208d872c6f0e597cc10b0701821b0000000b07810d11a3581c279c909f348e533da5808898f87f9a14bb2c3dfbbacccd631d927a3fa144534e454b1a043526c2581c4a27465112a39464e6dd5ee470c552ebb3cb42925d5ec04014967908a14c534e454b5f4144415f4e465401581c7bddf2c27f257eeeef3e892758b479e09c89a73642499797f2a97f3ca14b534e454b5f4144415f4c511b7fffffff939f34ff028201d81858bad8799fd8799f581c4a27465112a39464e6dd5ee470c552ebb3cb42925d5ec040149679084c534e454b5f4144415f4e4654ffd8799f4040ffd8799f581c279c909f348e533da5808898f87f9a14bb2c3dfbbacccd631d927a3f44534e454bffd8799f581c7bddf2c27f257eeeef3e892758b479e09c89a73642499797f2a97f3c4b534e454b5f4144415f4c51ff1903e59f581c856e34eac199979f7c04d4b500c6e91748dec14d92a28b3c1bf75882ff1b00000004a817c800ff";
}
