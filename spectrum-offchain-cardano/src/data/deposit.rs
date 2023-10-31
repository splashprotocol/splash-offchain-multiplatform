use cml_chain::plutus::PlutusData;
use cml_core::serialization::FromBytes;
use cml_crypto::Ed25519KeyHash;
use cml_multi_era::babbage::BabbageTransactionOutput;

use spectrum_cardano_lib::plutus_data::{
    ConstrPlutusDataExtension, DatumExtension, PlutusDataExtension, RequiresRedeemer,
};
use spectrum_cardano_lib::transaction::TransactionOutputExtension;
use spectrum_cardano_lib::types::TryFromPData;
use spectrum_cardano_lib::value::ValueExtension;
use spectrum_cardano_lib::{OutputRef, TaggedAmount, TaggedAssetClass};
use spectrum_offchain::data::UniqueOrder;
use spectrum_offchain::ledger::TryFromLedger;

use crate::constants::{ORDER_APPLY_RAW_REDEEMER, ORDER_REFUND_RAW_REDEEMER};
use crate::data::order::{ClassicalOrder, ClassicalOrderAction, PoolNft};
use crate::data::pool::CFMMPoolAction::Deposit as DepositAction;
use crate::data::pool::{CFMMPoolAction, Lq, Rx, Ry};
use crate::data::{OnChainOrderId, PoolId};

#[derive(Debug, Clone)]
pub struct Deposit {
    pub pool_nft: PoolId,
    pub token_x: TaggedAssetClass<Rx>,
    pub token_x_amount: TaggedAmount<Rx>,
    pub token_y: TaggedAssetClass<Ry>,
    pub token_y_amount: TaggedAmount<Ry>,
    pub token_lq: TaggedAssetClass<Lq>,
    pub ex_fee: u64,
    pub reward_pkh: Ed25519KeyHash,
    pub reward_stake_pkh: Option<Ed25519KeyHash>,
    pub collateral_ada: u64,
}

pub type ClassicalOnChainDeposit = ClassicalOrder<OnChainOrderId, Deposit>;

impl RequiresRedeemer<ClassicalOrderAction> for ClassicalOnChainDeposit {
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

impl Into<CFMMPoolAction> for ClassicalOnChainDeposit {
    fn into(self) -> CFMMPoolAction {
        DepositAction
    }
}

impl UniqueOrder for ClassicalOnChainDeposit {
    type TOrderId = OnChainOrderId;
    fn get_self_ref(&self) -> Self::TOrderId {
        self.id
    }
}

impl TryFromLedger<BabbageTransactionOutput, OutputRef> for ClassicalOnChainDeposit {
    fn try_from_ledger(repr: BabbageTransactionOutput, ctx: OutputRef) -> Option<Self> {
        let value = repr.value().clone();
        let conf = OnChainDepositConfig::try_from_pd(repr.clone().into_datum()?.into_pd()?)?;
        let token_x_amount = TaggedAmount::tag(value.amount_of(conf.token_x.untag()).unwrap_or(0));
        let token_y_amount = TaggedAmount::tag(value.amount_of(conf.token_y.untag()).unwrap_or(0));
        let deposit = Deposit {
            pool_nft: PoolId::try_from(conf.pool_nft).ok()?,
            token_x: conf.token_x,
            token_x_amount,
            token_y: conf.token_y,
            token_y_amount,
            token_lq: conf.token_lq,
            ex_fee: conf.ex_fee,
            reward_pkh: conf.reward_pkh,
            reward_stake_pkh: conf.reward_stake_pkh,
            collateral_ada: conf.collateral_ada,
        };

        Some(ClassicalOrder {
            id: OnChainOrderId::from(ctx),
            pool_id: PoolId::try_from(conf.pool_nft).ok()?,
            order: deposit,
        })
    }
}

struct OnChainDepositConfig {
    pool_nft: TaggedAssetClass<PoolNft>,
    token_x: TaggedAssetClass<Rx>,
    token_y: TaggedAssetClass<Ry>,
    token_lq: TaggedAssetClass<Lq>,
    ex_fee: u64,
    reward_pkh: Ed25519KeyHash,
    reward_stake_pkh: Option<Ed25519KeyHash>,
    collateral_ada: u64,
}

impl TryFromPData for OnChainDepositConfig {
    fn try_from_pd(data: PlutusData) -> Option<Self> {
        let mut cpd = data.into_constr_pd()?;
        let stake_pkh: Option<Ed25519KeyHash> = cpd
            .take_field(6)
            .and_then(|pd| pd.into_bytes())
            .and_then(|bytes| <[u8; 28]>::try_from(bytes).ok())
            .map(|bytes| Ed25519KeyHash::from(bytes));

        Some(OnChainDepositConfig {
            pool_nft: TaggedAssetClass::try_from_pd(cpd.take_field(0)?)?,
            token_x: TaggedAssetClass::try_from_pd(cpd.take_field(1)?)?,
            token_y: TaggedAssetClass::try_from_pd(cpd.take_field(2)?)?,
            token_lq: TaggedAssetClass::try_from_pd(cpd.take_field(3)?)?,
            ex_fee: cpd.take_field(4)?.into_u64()?,
            reward_pkh: Ed25519KeyHash::from(<[u8; 28]>::try_from(cpd.take_field(5)?.into_bytes()?).ok()?),
            reward_stake_pkh: stake_pkh,
            collateral_ada: cpd.take_field(7)?.into_u64()?,
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
    use cml_chain::Value;
    use cml_core::serialization::Deserialize;
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
    use crate::config::RefScriptsConfig;
    use crate::creds::operator_creds;
    use crate::data::deposit::OnChainDepositConfig;
    use crate::data::execution_context::ExecutionContext;
    use crate::data::order::ClassicalOnChainOrder;
    use crate::data::pool::CFMMPool;
    use crate::data::ref_scripts::RefScriptsOutputs;
    use crate::data::OnChain;

    #[test]
    fn parse_deposit_datum_mainnet() {
        let pd = PlutusData::from_cbor_bytes(&*hex::decode(DATUM_SAMPLE).unwrap()).unwrap();
        let maybe_conf = OnChainDepositConfig::try_from_pd(pd);
        assert!(maybe_conf.is_some())
    }

    const DATUM_SAMPLE: &str =
        "d8799fd8799f581ca80022230c821a52e426d2fdb096e7d967b5ab25d350d469a7603dbf4b5350465f4144415f4e4654ffd8799f4040ffd8799f581c09f2d4e4a5c3662f4c1e6a7d9600e9605279dbdcedb22d4507cb6e7543535046ffd8799f581c74f47c99ac793c29280575b08fe20c7fb75543bff5b1581f7c162e7c4a5350465f4144415f4c51ff1a0016e360581ca104c691610dccf8e15f156bb96923cce7d61cc74370ed8e9111ef05d8799f581c874ea2c1b94eed88c1d09f057e96ffe0fa6311527e2003432a968794ff1a0014c828ff";

    #[tokio::test]
    async fn run_valid_deposit_against_pool() {
        let deposit_ref = OutputRef::from((TransactionHash::from([0u8; 32]), 0));
        let pool_ref = OutputRef::from((TransactionHash::from([1u8; 32]), 0));
        let deposit_box =
            BabbageTransactionOutput::from_cbor_bytes(&*hex::decode(DEPOSIT_SAMPLE).unwrap()).unwrap();
        let pool_box =
            BabbageTransactionOutput::from_cbor_bytes(&*hex::decode(POOL_SAMPLE).unwrap()).unwrap();
        let deposit = ClassicalOnChainOrder::try_from_ledger(deposit_box, deposit_ref).unwrap();
        let pool = <OnChain<CFMMPool>>::try_from_ledger(pool_box, pool_ref).unwrap();

        let private_key_bech32 = Bip32PrivateKey::generate_ed25519_bip32().to_bech32();

        let (_, operator_pkh, operator_addr) =
            operator_creds(private_key_bech32.as_str(), NetworkInfo::mainnet());

        let test_address = EnterpriseAddress::new(
            NetworkInfo::mainnet().network_id(),
            StakeCredential::new_pub_key(operator_pkh),
        )
        .to_address();

        let collateral_outputs: TransactionOutput = TransactionOutput::new(
            test_address,
            Value::from(10000000),
            None,
            None, // todo: explorer doesn't support script ref. Change to correct after explorer update
        );

        let mock_requestor = MockBasedRequestor::new(collateral_outputs);

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

        let collateral = mock_requestor
            .get_collateral()
            .await
            .expect("Couldn't retrieve collateral");

        let ctx = ExecutionContext::new(operator_addr, ref_scripts, collateral);

        let result = pool.try_run(deposit, ctx);

        assert!(result.is_ok())
    }

    const DEPOSIT_SAMPLE: &str = "a300581d71075e09eb0fa89e1dc34691b3c56a7f437e60ac5ea67b338f2e176e2001821a57f316c6a1581c25c5de5f5b286073c593edfd77b48abc7a48e5a4f3d4cd9d428ff935a1434254431a00129beb028201d81858d5d8799fd8799f581cff63b385a615b3dce991f4dcf1ff7bd0082927e01e4e699c88ff7d994b4254435f4144415f4e4654ffd8799f4040ffd8799f581c25c5de5f5b286073c593edfd77b48abc7a48e5a4f3d4cd9d428ff93543425443ffd8799f581c4cf6a914372155ec78f6643141e280d38e3bae4ffbf623765efce3de4a4254435f4144415f4c51ff1a0016e360581c0f1b623e1d789ffea12b31d93418fa09b7ff2340805eb01cfe212491d8799f581c73fe9b3f0e63a8f460dba6af8b8daa5532491f1410cca937cc39960bff1a0014c828ff";
    const POOL_SAMPLE: &str = "a3005839316b9c456aa650cb808a9ab54326e039d5235ed69f069c9664a8fe5b69b2f6abf60ccde92eae1a2f4fdf65f2eaf6208d872c6f0e597cc10b0701821b000000ef71292ccea3581c25c5de5f5b286073c593edfd77b48abc7a48e5a4f3d4cd9d428ff935a1434254431a32c2ef90581c4cf6a914372155ec78f6643141e280d38e3bae4ffbf623765efce3dea14a4254435f4144415f4c511b7ffffff91d173162581cff63b385a615b3dce991f4dcf1ff7bd0082927e01e4e699c88ff7d99a14b4254435f4144415f4e465401028201d81858afd8799fd8799f581cff63b385a615b3dce991f4dcf1ff7bd0082927e01e4e699c88ff7d994b4254435f4144415f4e4654ffd8799f4040ffd8799f581c25c5de5f5b286073c593edfd77b48abc7a48e5a4f3d4cd9d428ff93543425443ffd8799f581c4cf6a914372155ec78f6643141e280d38e3bae4ffbf623765efce3de4a4254435f4144415f4c51ff1903e59f581c7b6e01b4ef327b039e11178f08369f1ce1861eb978880dcc0d9d6e74ff00ff";
}
