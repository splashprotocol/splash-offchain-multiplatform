use crate::data::order::PoolNft;
use crate::data::pool::{Lq, Rx, Ry};
use cml_chain::plutus::PlutusData;
use cml_crypto::ScriptHash;
use spectrum_cardano_lib::address::InlineCredential;
use spectrum_cardano_lib::plutus_data::ConstrPlutusDataExtension;
use spectrum_cardano_lib::plutus_data::PlutusDataExtension;
use spectrum_cardano_lib::types::TryFromPData;
use spectrum_cardano_lib::TaggedAssetClass;
use std::fmt::Debug;

#[derive(Debug)]
pub struct RoyaltyPoolConfig {
    pub pool_nft: TaggedAssetClass<PoolNft>,
    pub asset_x: TaggedAssetClass<Rx>,
    pub asset_y: TaggedAssetClass<Ry>,
    pub asset_lq: TaggedAssetClass<Lq>,
    pub lp_fee_num: u64,
    pub treasury_fee_num: u64,
    pub royalty_fee_num: u64,
    pub treasury_x: u64,
    pub treasury_y: u64,
    pub royalty_x: u64,
    pub royalty_y: u64,
    pub admin_address: ScriptHash,
    pub treasury_address: Vec<u8>,
    pub royalty_pub_key: Vec<u8>,
    pub nonce: u64,
}

pub struct RoyaltyPoolDatumMapping {
    pub pool_nft: usize,
    pub asset_x: usize,
    pub asset_y: usize,
    pub asset_lq: usize,
    pub lp_fee_num: usize,
    pub treasury_fee_num: usize,
    pub royalty_fee_num: usize,
    pub treasury_x: usize,
    pub treasury_y: usize,
    pub royalty_x: usize,
    pub royalty_y: usize,
    pub admin_address: usize,
    pub treasury_address: usize,
    pub royalty_pub_key: usize,
    pub royalty_nonce: usize,
}

pub const ROYALTY_DATUM_MAPPING: RoyaltyPoolDatumMapping = RoyaltyPoolDatumMapping {
    pool_nft: 0,
    asset_x: 1,
    asset_y: 2,
    asset_lq: 3,
    lp_fee_num: 4,
    treasury_fee_num: 5,
    royalty_fee_num: 6,
    treasury_x: 7,
    treasury_y: 8,
    royalty_x: 9,
    royalty_y: 10,
    admin_address: 11,
    treasury_address: 12,
    royalty_pub_key: 13,
    royalty_nonce: 14,
};

impl TryFromPData for RoyaltyPoolConfig {
    fn try_from_pd(data: PlutusData) -> Option<Self> {
        let mut cpd = data.into_constr_pd()?;

        let admin_address = cpd
            .clone()
            .take_field(ROYALTY_DATUM_MAPPING.admin_address)
            .and_then(TryFromPData::try_from_pd)
            .and_then(|creds: Vec<InlineCredential>| {
                creds.first().and_then(|cred| cred.clone().script_hash())
            })?;

        Some(Self {
            pool_nft: TaggedAssetClass::try_from_pd(cpd.take_field(ROYALTY_DATUM_MAPPING.pool_nft)?)?,
            asset_x: TaggedAssetClass::try_from_pd(cpd.take_field(ROYALTY_DATUM_MAPPING.asset_x)?)?,
            asset_y: TaggedAssetClass::try_from_pd(cpd.take_field(ROYALTY_DATUM_MAPPING.asset_y)?)?,
            asset_lq: TaggedAssetClass::try_from_pd(cpd.take_field(ROYALTY_DATUM_MAPPING.asset_lq)?)?,
            lp_fee_num: cpd.take_field(ROYALTY_DATUM_MAPPING.lp_fee_num)?.into_u64()?,
            treasury_fee_num: cpd
                .take_field(ROYALTY_DATUM_MAPPING.treasury_fee_num)?
                .into_u64()?,
            royalty_fee_num: cpd
                .take_field(ROYALTY_DATUM_MAPPING.royalty_fee_num)?
                .into_u64()?,
            treasury_x: cpd.take_field(ROYALTY_DATUM_MAPPING.treasury_x)?.into_u64()?,
            treasury_y: cpd.take_field(ROYALTY_DATUM_MAPPING.treasury_y)?.into_u64()?,
            royalty_x: cpd.take_field(ROYALTY_DATUM_MAPPING.royalty_x)?.into_u64()?,
            royalty_y: cpd.take_field(ROYALTY_DATUM_MAPPING.royalty_y)?.into_u64()?,
            admin_address: admin_address,
            treasury_address: cpd
                .take_field(ROYALTY_DATUM_MAPPING.treasury_address)?
                .into_bytes()?,
            royalty_pub_key: cpd
                .take_field(ROYALTY_DATUM_MAPPING.royalty_pub_key)?
                .into_bytes()?,
            nonce: cpd.take_field(ROYALTY_DATUM_MAPPING.royalty_nonce)?.into_u64()?,
        })
    }
}

mod tests {
    use crate::creds::OperatorCred;
    use crate::data::cfmm_pool::ConstFnPool;
    use crate::data::order::Order::RoyaltyWithdraw;
    use crate::data::pool::{AnyPool, PoolValidation};
    use crate::data::royalty_pool::RoyaltyPoolConfig;
    use crate::data::royalty_withdraw_request::OnChainRoyaltyWithdraw;
    use crate::deployment::ProtocolValidator::{
        ConstFnPoolFeeSwitch, ConstFnPoolFeeSwitchBiDirFee, ConstFnPoolFeeSwitchV2, ConstFnPoolV1,
        ConstFnPoolV2, LimitOrderV1, RoyaltyPoolV1, RoyaltyPoolV1RoyaltyWithdrawRequest,
    };
    use crate::deployment::{DeployedScriptInfo, DeployedValidators, ProtocolScriptHashes};
    use crate::handler_context::{ConsumedIdentifiers, ConsumedInputs, ProducedIdentifiers};
    use cml_chain::transaction::TransactionOutput;
    use cml_core::serialization::Deserialize;
    use cml_crypto::{Ed25519KeyHash, TransactionHash};
    use spectrum_cardano_lib::{OutputRef, Token};
    use spectrum_offchain::data::small_set::SmallVec;
    use spectrum_offchain::domain::Has;
    use spectrum_offchain::ledger::TryFromLedger;
    use type_equalities::IsEqual;

    struct Context {
        oref: OutputRef,
        royalty_pool: DeployedScriptInfo<{ RoyaltyPoolV1 as u8 }>,
        const_fn_pool_v1: DeployedScriptInfo<{ ConstFnPoolV1 as u8 }>,
        const_fn_pool_v2: DeployedScriptInfo<{ ConstFnPoolV2 as u8 }>,
        fee_switch_v1: DeployedScriptInfo<{ ConstFnPoolFeeSwitch as u8 }>,
        fee_switch_v2: DeployedScriptInfo<{ ConstFnPoolFeeSwitchV2 as u8 }>,
        fee_switch_bi_dir: DeployedScriptInfo<{ ConstFnPoolFeeSwitchBiDirFee as u8 }>,
        royalty_withdraw: DeployedScriptInfo<{ RoyaltyPoolV1RoyaltyWithdrawRequest as u8 }>,
        cred: OperatorCred,
        consumed_inputs: ConsumedInputs,
        consumed_identifiers: ConsumedIdentifiers<Token>,
        produced_identifiers: ProducedIdentifiers<Token>,
        pool_validation: PoolValidation,
    }

    impl Has<DeployedScriptInfo<{ RoyaltyPoolV1 as u8 }>> for Context {
        fn select<U: IsEqual<DeployedScriptInfo<{ RoyaltyPoolV1 as u8 }>>>(
            &self,
        ) -> DeployedScriptInfo<{ RoyaltyPoolV1 as u8 }> {
            self.royalty_pool
        }
    }

    impl Has<DeployedScriptInfo<{ ConstFnPoolV1 as u8 }>> for Context {
        fn select<U: IsEqual<DeployedScriptInfo<{ ConstFnPoolV1 as u8 }>>>(
            &self,
        ) -> DeployedScriptInfo<{ ConstFnPoolV1 as u8 }> {
            self.const_fn_pool_v1
        }
    }

    impl Has<DeployedScriptInfo<{ ConstFnPoolV2 as u8 }>> for Context {
        fn select<U: IsEqual<DeployedScriptInfo<{ ConstFnPoolV2 as u8 }>>>(
            &self,
        ) -> DeployedScriptInfo<{ ConstFnPoolV2 as u8 }> {
            self.const_fn_pool_v2
        }
    }

    impl Has<DeployedScriptInfo<{ ConstFnPoolFeeSwitch as u8 }>> for Context {
        fn select<U: IsEqual<DeployedScriptInfo<{ ConstFnPoolFeeSwitch as u8 }>>>(
            &self,
        ) -> DeployedScriptInfo<{ ConstFnPoolFeeSwitch as u8 }> {
            self.fee_switch_v1
        }
    }

    impl Has<DeployedScriptInfo<{ ConstFnPoolFeeSwitchV2 as u8 }>> for Context {
        fn select<U: IsEqual<DeployedScriptInfo<{ ConstFnPoolFeeSwitchV2 as u8 }>>>(
            &self,
        ) -> DeployedScriptInfo<{ ConstFnPoolFeeSwitchV2 as u8 }> {
            self.fee_switch_v2
        }
    }

    impl Has<DeployedScriptInfo<{ ConstFnPoolFeeSwitchBiDirFee as u8 }>> for Context {
        fn select<U: IsEqual<DeployedScriptInfo<{ ConstFnPoolFeeSwitchBiDirFee as u8 }>>>(
            &self,
        ) -> DeployedScriptInfo<{ ConstFnPoolFeeSwitchBiDirFee as u8 }> {
            self.fee_switch_bi_dir
        }
    }

    impl Has<DeployedScriptInfo<{ RoyaltyPoolV1RoyaltyWithdrawRequest as u8 }>> for Context {
        fn select<U: IsEqual<DeployedScriptInfo<{ RoyaltyPoolV1RoyaltyWithdrawRequest as u8 }>>>(
            &self,
        ) -> DeployedScriptInfo<{ RoyaltyPoolV1RoyaltyWithdrawRequest as u8 }> {
            self.royalty_withdraw
        }
    }

    impl Has<OutputRef> for Context {
        fn select<U: IsEqual<OutputRef>>(&self) -> OutputRef {
            self.oref
        }
    }

    impl Has<PoolValidation> for Context {
        fn select<U: IsEqual<PoolValidation>>(&self) -> PoolValidation {
            self.pool_validation
        }
    }

    const POOL_UTXO: &str = "a300581d7040a533ea4e3023c62912f029c7ad388bf3c2254e9c7fb3450024bc6e011a00989680028201d8185901a2d87982d87986d87982581cc7b0bf7947ef297f5f2f1724d35366a3d4e370b2dbe528271818f1174b4352595f4144415f4e46541a004c4b4019c350581c3132477f91c2f31da7d9b4eeddf0fd7c74e91fdc13770c99747fa04958203b6b7154e01955cdc0869191b5e9c53148df89a4f137442ad3f09a21175aeb7f1a0044aa205f5840845846a201276761646472657373583900719bee424a97b58b3dca88fe5da6feac6494aa7226f975f3506c5b257846f6bb07f5b2825885e4502679e699b4e60a58400c4609a46bc35454cda166686173686564f45881d87982d87986d87982581cc7b0bf7947ef297f5f2f1724d35366a3d4e370b2dbe528271818f1174b4352595f58404144415f4e46541a004c4b4019c350581c3132477f91c2f31da7d9b4eeddf0fd7c74e91fdc13770c99747fa04958203b6b7154e01955cdc0869191b5e9c531485840df89a4f137442ad3f09a21175aeb7f1a0044aa20015840b3fd2260a34ce943473142b970afbbc6297b04eb5551431a95abc6a7cdf028c6c4b04daf537b41046b57c29d7e5a12471823a3a4c44c3cde29416b1f5518a6c70fff";

    #[test]
    fn try_read() {
        const TX: &str = "a035c1cb245735680dcb3c46a9a3e692fbf550c8a5d7c4ada1471f97cc92dc55";
        const IX: u64 = 0;
        const ORDER_IX: u64 = 0;
        let oref = OutputRef::new(TransactionHash::from_hex(TX).unwrap(), IX);
        let raw_deployment = std::fs::read_to_string("/Users/aleksandr/IdeaProjects/spectrum-offchain-multiplatform/bloom-cardano-agent/resources/preprod.deployment.json").expect("Cannot load deployment file");
        let deployment: DeployedValidators =
            serde_json::from_str(&raw_deployment).expect("Invalid deployment file");
        let scripts = ProtocolScriptHashes::from(&deployment);
        let ctx = Context {
            oref,
            royalty_pool: scripts.royalty_pool_v1,
            const_fn_pool_v1: scripts.const_fn_pool_v1,
            const_fn_pool_v2: scripts.const_fn_pool_v2,
            fee_switch_v1: scripts.const_fn_pool_fee_switch,
            fee_switch_v2: scripts.const_fn_pool_fee_switch_v2,
            fee_switch_bi_dir: scripts.const_fn_pool_fee_switch_bidir_fee,
            royalty_withdraw: scripts.royalty_pool_withdraw_request,
            cred: OperatorCred(Ed25519KeyHash::from([0u8; 28])),
            consumed_inputs: SmallVec::new(vec![oref].into_iter()).into(),
            consumed_identifiers: Default::default(),
            produced_identifiers: Default::default(),
            pool_validation: PoolValidation {
                min_n2t_lovelace: 10,
                min_t2t_lovelace: 10,
            },
        };
        let bearer = TransactionOutput::from_cbor_bytes(&*hex::decode(POOL_UTXO).unwrap()).unwrap();
        let pool = OnChainRoyaltyWithdraw::try_from_ledger(&bearer, &ctx);
        assert_eq!(pool.is_some(), true)
    }
}
