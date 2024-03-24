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
use spectrum_offchain::data::order::UniqueOrder;
use spectrum_offchain::data::Has;
use spectrum_offchain::ledger::TryFromLedger;

use crate::constants::{
    DEPOSIT_SCRIPT_V2, ORDER_APPLY_RAW_REDEEMER, ORDER_APPLY_RAW_REDEEMER_V2, ORDER_REFUND_RAW_REDEEMER,
};
use crate::data::limit_swap::ClassicalOnChainLimitSwap;
use crate::data::order::{ClassicalOrder, ClassicalOrderAction, PoolNft};
use crate::data::pool::CFMMPoolAction::Deposit as DepositAction;
use crate::data::pool::{CFMMPoolAction, Lq, Rx, Ry};
use crate::data::{OnChainOrderId, PoolId};
use crate::deployment::ProtocolValidator::ConstFnPoolDeposit;
use crate::deployment::{DeployedValidator, DeployedValidatorErased, RequiresValidator};

#[derive(Debug, Clone, Eq, PartialEq)]
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

impl<Ctx> RequiresValidator<Ctx> for ClassicalOnChainDeposit
where
    Ctx: Has<DeployedValidator<{ ConstFnPoolDeposit as u8 }>>,
{
    fn get_validator(&self, ctx: &Ctx) -> DeployedValidatorErased {
        ctx.get().erased()
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
    fn try_from_ledger(repr: &BabbageTransactionOutput, ctx: OutputRef) -> Option<Self> {
        if repr.address().to_bech32(None).unwrap() != DEPOSIT_SCRIPT_V2 {
            return None;
        }
        let value = repr.value().clone();
        let conf = OnChainDepositConfig::try_from_pd(repr.clone().into_datum()?.into_pd()?)?;
        let token_x_amount = TaggedAmount::new(value.amount_of(conf.token_x.untag()).unwrap_or(0));
        let token_y_amount = TaggedAmount::new(value.amount_of(conf.token_y.untag()).unwrap_or(0));
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
            .clone()
            .and_then(|pd| pd.into_constr_pd())
            .and_then(|mut cpd_spkh| cpd_spkh.take_field(0))
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
    use crate::creds::operator_creds;
    use crate::data::deposit::OnChainDepositConfig;
    use crate::data::execution_context::ExecutionContext;
    use crate::data::order::ClassicalOnChainOrder;
    use crate::data::pool::CFMMPool;
    use crate::data::ref_scripts::ReferenceOutputs;
    use crate::data::OnChain;
    use crate::ref_scripts::ReferenceSources;

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
        let deposit = ClassicalOnChainOrder::try_from_ledger(&deposit_box, deposit_ref).unwrap();
        let pool = <OnChain<CFMMPool>>::try_from_ledger(&pool_box, pool_ref).unwrap();

        let private_key_bech32 = Bip32PrivateKey::generate_ed25519_bip32().to_bech32();

        let (_, operator_pkh, operator_addr) = operator_creds(private_key_bech32.as_str(), 1);

        let test_address = EnterpriseAddress::new(
            NetworkInfo::testnet().network_id(),
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

        let ctx = ExecutionContext::new(operator_addr, ref_scripts, collateral, 1);

        let result = pool.try_run(deposit, ctx);
        assert!(result.is_ok())
    }

    const DEPOSIT_SAMPLE: &str = "a300581d7059153a6a2927dcdbd8d81deb3520247c4195605fe044bf60d514e78c01821a003c2a36a1581c4b3459fd18a1dbabe207cd19c9951a9fac9f5c0f9c384e3d97efba26a14574657374421a3b9aca00028201d81858dbd8799fd8799f581c25188bfd0ec2da7b29fad72a37f65ca27aefd50cf4a44d7191068e564d74657374425f4144415f4e4654ffd8799f4040ffd8799f581c4b3459fd18a1dbabe207cd19c9951a9fac9f5c0f9c384e3d97efba26457465737442ffd8799f581cd7209684f1c0460e25e8f8950946c6b69353c3d8ebe80617aeff11364c74657374425f4144415f4c51ff1a0016e360581c24ce811ca3f9afc83a6b35dc1de4017a8b7f4199d95e1dcf3b51d3fbd8799f581c906dfeb836d6597f1e7f9f86a3143ba3490fd72eee1683832c5b8224ff1a00150b80ff";
    const POOL_SAMPLE: &str = "a300581d70e4259405278073a7ca5afcc006baa5a52c0c66bfc47b75e9a8c8277901821a7e203eeba3581c4b3459fd18a1dbabe207cd19c9951a9fac9f5c0f9c384e3d97efba26a14574657374421b000001cf257827ae581cd7209684f1c0460e25e8f8950946c6b69353c3d8ebe80617aeff1136a14c74657374425f4144415f4c511b7ffffff0e512ba44581c25188bfd0ec2da7b29fad72a37f65ca27aefd50cf4a44d7191068e56a14d74657374425f4144415f4e465401028201d81858b9d8799fd8799f581c25188bfd0ec2da7b29fad72a37f65ca27aefd50cf4a44d7191068e564d74657374425f4144415f4e4654ffd8799f4040ffd8799f581c4b3459fd18a1dbabe207cd19c9951a9fac9f5c0f9c384e3d97efba26457465737442ffd8799f581cd7209684f1c0460e25e8f8950946c6b69353c3d8ebe80617aeff11364c74657374425f4144415f4c51ff19270d01091917708000581c2618e94cdb06792f05ae9b1ec78b0231f4b7f4215b1b4cf52e6342deff";
}
