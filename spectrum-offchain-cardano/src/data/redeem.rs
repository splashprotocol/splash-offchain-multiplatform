use cml_chain::plutus::PlutusData;
use cml_core::serialization::FromBytes;
use cml_crypto::Ed25519KeyHash;
use cml_multi_era::babbage::BabbageTransactionOutput;

use spectrum_cardano_lib::{AssetClass, OutputRef, TaggedAmount, TaggedAssetClass};
use spectrum_cardano_lib::plutus_data::{
    ConstrPlutusDataExtension, DatumExtension, PlutusDataExtension, RequiresRedeemer,
};
use spectrum_cardano_lib::transaction::TransactionOutputExtension;
use spectrum_cardano_lib::types::TryFromPData;
use spectrum_cardano_lib::value::ValueExtension;
use spectrum_offchain::data::order::UniqueOrder;
use spectrum_offchain::ledger::TryFromLedger;

use crate::constants::{
    ORDER_APPLY_RAW_REDEEMER, ORDER_APPLY_RAW_REDEEMER_V2, ORDER_REFUND_RAW_REDEEMER, REDEEM_SCRIPT_V2,
};
use crate::data::{OnChainOrderId, PoolId};
use crate::data::order::{ClassicalOrder, ClassicalOrderAction, PoolNft};
use crate::data::pool::{CFMMPoolAction, Lq, OrderInputIdx, Rx, Ry};
use crate::data::pool::CFMMPoolAction::Redeem as RedeemAction;

#[derive(Debug, Clone, Eq, PartialEq)]
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

impl RequiresRedeemer<(ClassicalOrderAction, OrderInputIdx)> for ClassicalOnChainRedeem {
    fn redeemer(action: (ClassicalOrderAction, OrderInputIdx)) -> PlutusData {
        match action {
            (ClassicalOrderAction::Apply, OrderInputIdx::Idx0) => {
                PlutusData::from_bytes(hex::decode(ORDER_APPLY_RAW_REDEEMER_V2).unwrap()).unwrap()
            }
            (ClassicalOrderAction::Apply, OrderInputIdx::Idx1) => {
                PlutusData::from_bytes(hex::decode(ORDER_APPLY_RAW_REDEEMER).unwrap()).unwrap()
            }
            (ClassicalOrderAction::Refund, _) => {
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
    fn try_from_ledger(repr: &BabbageTransactionOutput, ctx: OutputRef) -> Option<Self> {
        if repr.address().to_bech32(None).unwrap() != REDEEM_SCRIPT_V2 {
            return None;
        }
        let value = repr.value().clone();
        let conf = OnChainRedeemConfig::try_from_pd(repr.datum().clone()?.into_pd()?)?;
        let token_lq_amount = TaggedAmount::new(value.amount_of(conf.token_lq.untag()).unwrap_or(0));
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
            .clone()
            .and_then(|pd| pd.into_constr_pd())
            .and_then(|mut cpd_spkh| cpd_spkh.take_field(0))
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
    use spectrum_cardano_lib::OutputRef;
    use spectrum_cardano_lib::types::TryFromPData;
    use spectrum_offchain::executor::RunOrder;
    use spectrum_offchain::ledger::TryFromLedger;

    use crate::collaterals::Collaterals;
    use crate::collaterals::tests::MockBasedRequestor;
    use crate::creds::operator_creds;
    use crate::data::execution_context::ExecutionContext;
    use crate::data::OnChain;
    use crate::data::order::ClassicalOnChainOrder;
    use crate::data::pool::CFMMPool;
    use crate::data::redeem::OnChainRedeemConfig;
    use crate::data::ref_scripts::ReferenceOutputs;
    use crate::ref_scripts::ReferenceSources;

    #[test]
    fn parse_deposit_datum_mainnet() {
        let pd = PlutusData::from_cbor_bytes(&*hex::decode(DATUM_SAMPLE).unwrap()).unwrap();
        let maybe_conf = OnChainRedeemConfig::try_from_pd(pd);
        assert!(maybe_conf.is_some())
    }

    const DATUM_SAMPLE: &str =
        "d8799fd8799f581cbb461a9afa6e60962c72d520b476f60f5b24554614531ef1fe34236853436f726e75636f706961735f4144415f4e4654ffd8799f4040ffd8799f581cb6a7467ea1deb012808ef4e87b5ff371e85f7142d7b356a40d9b42a0581e436f726e75636f70696173205b76696120436861696e506f72742e696f5dffd8799f581ce6cdb6e0e98a136df23bbea57ab39417c82302947779be2d9acedf0a52436f726e75636f706961735f4144415f4c51ff1a0016e360581cf197ea0891ce786a9a41b59255bf0efa6c2fb47d0d0babdfed7a294cd8799f581c0a391e83011b5bcfdc7435e9b50fbff6a8bdeb9e7ad8706f7b2673dbffff";

    #[tokio::test]
    async fn run_valid_redeem_against_pool() {
        let redeem_ref = OutputRef::from((TransactionHash::from([0u8; 32]), 0));
        let pool_ref = OutputRef::from((TransactionHash::from([1u8; 32]), 0));
        let redeem_box =
            BabbageTransactionOutput::from_cbor_bytes(&*hex::decode(REDEEM_SAMPLE).unwrap()).unwrap();
        let pool_box =
            BabbageTransactionOutput::from_cbor_bytes(&*hex::decode(POOL_SAMPLE).unwrap()).unwrap();
        let redeem = ClassicalOnChainOrder::try_from_ledger(&redeem_box, redeem_ref).unwrap();
        let pool = <OnChain<CFMMPool>>::try_from_ledger(&pool_box, pool_ref).unwrap();

        let private_key_bech32 = Bip32PrivateKey::generate_ed25519_bip32().to_bech32();

        let (_, operator_pkh, operator_addr) = operator_creds(
            private_key_bech32.as_str(),
            NetworkInfo::testnet().network_id().into(),
        );

        let test_address = EnterpriseAddress::new(
            NetworkInfo::testnet().network_id(),
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

        let explorer = Explorer::new(
            ExplorerConfig {
                url: "https://explorer.spectrum.fi",
            },
            u32::from(NetworkInfo::testnet().protocol_magic()) as u64,
        );

        let ref_scripts_conf = ReferenceSources {
            pool_v1_script: "8f7625e22caebbb6e082232d39a673e1ac73ae4c4abc747b6ccea9f59316d620#0"
                .try_into()
                .unwrap(),
            pool_v2_script: "c75d08f44ec13ea1f78ee96b7fcb94e3303f1812d3d38dcaeca341d8e146ac98#0"
                .try_into()
                .unwrap(),
            fee_switch_pool_script: "8f7625e22caebbb6e082232d39a673e1ac73ae4c4abc747b6ccea9f59316d620#0"
                .to_string()
                .try_into()
                .unwrap(),
            fee_switch_pool_bidirectional_fee_script:
                "c75d08f44ec13ea1f78ee96b7fcb94e3303f1812d3d38dcaeca341d8e146ac98#0"
                    .to_string()
                    .try_into()
                    .unwrap(),
            swap_script: "cb4d21dc2cd5471e4c3b3d2125339dcc0577465e9c9fb789636046b174c2a84d#1"
                .try_into()
                .unwrap(),
            deposit_script: "314ff562be2a5670827f42eb81457c827256378b44a59c9fdc13c2b78e67cab3#0"
                .try_into()
                .unwrap(),
            redeem_script: "cb4d21dc2cd5471e4c3b3d2125339dcc0577465e9c9fb789636046b174c2a84d#0"
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

        let ctx = ExecutionContext::new(
            operator_addr,
            ref_scripts,
            collateral,
            NetworkInfo::testnet().network_id().into(),
        );

        let result = pool.try_run(redeem, ctx);

        assert!(result.is_ok())
    }

    const REDEEM_SAMPLE: &str = "a300581d7046a81d17db9f751bf2c9deb3ae5814ee040b9938b1f172dc3b6eb68301821a001f1b66a1581ccaf179cc7a338121b7bcd9e7015d009399169d372d14e2aa16b8b283a14c74657374425f4144415f4c511a008e3d3f028201d81858d6d8799fd8799f581c0303293c397a0d0fa962fd891ab285333ade6d15c014c6c9f067e5984d74657374425f4144415f4e4654ffd8799f4040ffd8799f581c4b3459fd18a1dbabe207cd19c9951a9fac9f5c0f9c384e3d97efba26457465737442ffd8799f581ccaf179cc7a338121b7bcd9e7015d009399169d372d14e2aa16b8b2834c74657374425f4144415f4c51ff1a0016e360581c24ce811ca3f9afc83a6b35dc1de4017a8b7f4199d95e1dcf3b51d3fbd8799f581c906dfeb836d6597f1e7f9f86a3143ba3490fd72eee1683832c5b8224ffff";
    const POOL_SAMPLE: &str = "a300581d7076930108a2d528273fc1db3dad61f4604cdcbb46accb8bbefe157efc01821b0000000229b91578a3581c4b3459fd18a1dbabe207cd19c9951a9fac9f5c0f9c384e3d97efba26a14574657374421a0013a808581ccaf179cc7a338121b7bcd9e7015d009399169d372d14e2aa16b8b283a14c74657374425f4144415f4c511b7ffffffff97bc99c581c0303293c397a0d0fa962fd891ab285333ade6d15c014c6c9f067e598a14d74657374425f4144415f4e465401028201d81858b8d8799fd8799f581c0303293c397a0d0fa962fd891ab285333ade6d15c014c6c9f067e5984d74657374425f4144415f4e4654ffd8799f4040ffd8799f581c4b3459fd18a1dbabe207cd19c9951a9fac9f5c0f9c384e3d97efba26457465737442ffd8799f581ccaf179cc7a338121b7bcd9e7015d009399169d372d14e2aa16b8b2834c74657374425f4144415f4c51ff19270b011853008000581c2618e94cdb06792f05ae9b1ec78b0231f4b7f4215b1b4cf52e6342deff";
}
