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
use spectrum_cardano_lib::{AssetClass, OutputRef, TaggedAmount, TaggedAssetClass};
use spectrum_offchain::data::UniqueOrder;
use spectrum_offchain::ledger::TryFromLedger;

use crate::constants::{ORDER_APPLY_RAW_REDEEMER, ORDER_REFUND_RAW_REDEEMER};
use crate::data::order::{ClassicalOrder, ClassicalOrderAction, PoolNft};
use crate::data::pool::CFMMPoolAction::Redeem as RedeemAction;
use crate::data::pool::{CFMMPoolAction, Lq, Rx, Ry};
use crate::data::{OnChainOrderId, PoolId};

#[derive(Debug, Clone)]
pub struct Redeem {
    pub pool_nft: PoolId,
    pub token_x: TaggedAssetClass<Rx>,
    pub token_y: TaggedAssetClass<Ry>,
    pub token_lq: TaggedAssetClass<Lq>,
    pub token_lq_amount: TaggedAmount<Lq>,
    pub ex_fee: u64,
    pub reward_pkh: Ed25519KeyHash,
    pub reward_stake_pkh: Option<Ed25519KeyHash>,
    pub collateral_ada: u64,
}

pub type ClassicalOnChainRedeem = ClassicalOrder<OnChainOrderId, Redeem>;

impl RequiresRedeemer<ClassicalOrderAction> for ClassicalOnChainRedeem {
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

impl Into<CFMMPoolAction> for ClassicalOnChainRedeem {
    fn into(self) -> CFMMPoolAction {
        RedeemAction
    }
}

impl UniqueOrder for ClassicalOnChainRedeem {
    type TOrderId = OnChainOrderId;
    fn get_self_ref(&self) -> Self::TOrderId {
        self.id
    }
}

struct OnChainRedeemConfig {
    pool_nft: TaggedAssetClass<PoolNft>,
    token_x: TaggedAssetClass<Rx>,
    token_y: TaggedAssetClass<Ry>,
    token_lq: TaggedAssetClass<Lq>,
    ex_fee: u64,
    reward_pkh: Ed25519KeyHash,
    reward_stake_pkh: Option<Ed25519KeyHash>,
}

impl TryFromLedger<BabbageTransactionOutput, OutputRef> for ClassicalOnChainRedeem {
    fn try_from_ledger(repr: BabbageTransactionOutput, ctx: OutputRef) -> Option<Self> {
        let value = repr.value().clone();
        let conf = OnChainRedeemConfig::try_from_pd(repr.clone().into_datum()?.into_pd()?)?;
        let token_lq_amount = TaggedAmount::tag(value.amount_of(conf.token_lq.untag()).unwrap_or(0));
        let collateral_ada = value.amount_of(AssetClass::Native).unwrap_or(0) - conf.ex_fee;
        let redeem = Redeem {
            pool_nft: PoolId::try_from(conf.pool_nft).ok()?,
            token_x: conf.token_x,
            token_y: conf.token_y,
            token_lq: conf.token_lq,
            token_lq_amount,
            ex_fee: conf.ex_fee,
            reward_pkh: conf.reward_pkh,
            reward_stake_pkh: conf.reward_stake_pkh,
            collateral_ada,
        };

        Some(ClassicalOrder {
            id: OnChainOrderId::from(ctx),
            pool_id: PoolId::try_from(conf.pool_nft).ok()?,
            order: redeem,
        })
    }
}

impl TryFromPData for OnChainRedeemConfig {
    fn try_from_pd(data: PlutusData) -> Option<Self> {
        let mut cpd = data.into_constr_pd()?;
        let stake_pkh: Option<Ed25519KeyHash> = cpd
            .take_field(6)
            .and_then(|pd| pd.into_bytes())
            .and_then(|bytes| <[u8; 28]>::try_from(bytes).ok())
            .map(|bytes| Ed25519KeyHash::from(bytes));

        Some(OnChainRedeemConfig {
            pool_nft: TaggedAssetClass::try_from_pd(cpd.take_field(0)?)?,
            token_x: TaggedAssetClass::try_from_pd(cpd.take_field(1)?)?,
            token_y: TaggedAssetClass::try_from_pd(cpd.take_field(2)?)?,
            token_lq: TaggedAssetClass::try_from_pd(cpd.take_field(3)?)?,
            ex_fee: cpd.take_field(4)?.into_u64()?,
            reward_pkh: Ed25519KeyHash::from(<[u8; 28]>::try_from(cpd.take_field(5)?.into_bytes()?).ok()?),
            reward_stake_pkh: stake_pkh,
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
    use crate::data::execution_context::ExecutionContext;
    use crate::data::order::ClassicalOnChainOrder;
    use crate::data::pool::CFMMPool;
    use crate::data::redeem::OnChainRedeemConfig;
    use crate::data::ref_scripts::RefScriptsOutputs;
    use crate::data::OnChain;

    #[test]
    fn parse_deposit_datum_mainnet() {
        let pd = PlutusData::from_cbor_bytes(&*hex::decode(DATUM_SAMPLE).unwrap()).unwrap();
        let maybe_conf = OnChainRedeemConfig::try_from_pd(pd);
        assert!(maybe_conf.is_some())
    }

    const DATUM_SAMPLE: &str =
        "d8799fd8799f581cbb461a9afa6e60962c72d520b476f60f5b24554614531ef1fe34236853436f726e75636f706961735f4144415f4e4654ffd8799f4040ffd8799f581cb6a7467ea1deb012808ef4e87b5ff371e85f7142d7b356a40d9b42a0581e436f726e75636f70696173205b76696120436861696e506f72742e696f5dffd8799f581ce6cdb6e0e98a136df23bbea57ab39417c82302947779be2d9acedf0a52436f726e75636f706961735f4144415f4c51ff1a0016e360581cf197ea0891ce786a9a41b59255bf0efa6c2fb47d0d0babdfed7a294cd8799f581c0a391e83011b5bcfdc7435e9b50fbff6a8bdeb9e7ad8706f7b2673dbffff";

    #[tokio::test]
    async fn run_valid_deposit_against_pool() {
        let redeem_ref = OutputRef::from((TransactionHash::from([0u8; 32]), 0));
        let pool_ref = OutputRef::from((TransactionHash::from([1u8; 32]), 0));
        let redeem_box =
            BabbageTransactionOutput::from_cbor_bytes(&*hex::decode(REDEEM_SAMPLE).unwrap()).unwrap();
        let pool_box =
            BabbageTransactionOutput::from_cbor_bytes(&*hex::decode(POOL_SAMPLE).unwrap()).unwrap();
        let redeem = ClassicalOnChainOrder::try_from_ledger(redeem_box, redeem_ref).unwrap();
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

        let result = pool.try_run(redeem, ctx);

        assert!(result.is_ok())
    }

    const REDEEM_SAMPLE: &str = "a300581d7183da79f531c19f9ce4d85359f56968a742cf05cc25ed3ca48c302dee01821a0022002ea1581ce6cdb6e0e98a136df23bbea57ab39417c82302947779be2d9acedf0aa152436f726e75636f706961735f4144415f4c511a2202f3e1028201d81858fcd8799fd8799f581cbb461a9afa6e60962c72d520b476f60f5b24554614531ef1fe34236853436f726e75636f706961735f4144415f4e4654ffd8799f4040ffd8799f581cb6a7467ea1deb012808ef4e87b5ff371e85f7142d7b356a40d9b42a0581e436f726e75636f70696173205b76696120436861696e506f72742e696f5dffd8799f581ce6cdb6e0e98a136df23bbea57ab39417c82302947779be2d9acedf0a52436f726e75636f706961735f4144415f4c51ff1a0016e360581cf197ea0891ce786a9a41b59255bf0efa6c2fb47d0d0babdfed7a294cd8799f581c0a391e83011b5bcfdc7435e9b50fbff6a8bdeb9e7ad8706f7b2673dbffff";
    const POOL_SAMPLE: &str = "a3005839316b9c456aa650cb808a9ab54326e039d5235ed69f069c9664a8fe5b69b2f6abf60ccde92eae1a2f4fdf65f2eaf6208d872c6f0e597cc10b0701821b00000001ce4529f0a3581cb6a7467ea1deb012808ef4e87b5ff371e85f7142d7b356a40d9b42a0a1581e436f726e75636f70696173205b76696120436861696e506f72742e696f5d1b000000133b761eea581cbb461a9afa6e60962c72d520b476f60f5b24554614531ef1fe342368a153436f726e75636f706961735f4144415f4e465401581ce6cdb6e0e98a136df23bbea57ab39417c82302947779be2d9acedf0aa152436f726e75636f706961735f4144415f4c511b7ffffffa1d4dcb53028201d81858dbd8799fd8799f581cbb461a9afa6e60962c72d520b476f60f5b24554614531ef1fe34236853436f726e75636f706961735f4144415f4e4654ffd8799f4040ffd8799f581cb6a7467ea1deb012808ef4e87b5ff371e85f7142d7b356a40d9b42a0581e436f726e75636f70696173205b76696120436861696e506f72742e696f5dffd8799f581ce6cdb6e0e98a136df23bbea57ab39417c82302947779be2d9acedf0a52436f726e75636f706961735f4144415f4c51ff1903e59f581ce02b856d135aa3d92ce87c1b8128ecc1c773966995430f0698b09badff00ff";
}
