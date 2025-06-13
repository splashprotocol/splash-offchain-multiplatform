use crate::constants::FEE_DEN;
use crate::data::order::{ClassicalOrder, PoolNft};
use crate::data::pool::CFMMPoolAction::DAOAction;
use crate::data::pool::{CFMMPoolAction, Rx, Ry};
use crate::data::{OnChainOrderId, PoolId};
use crate::deployment::ProtocolValidator::RoyaltyPoolDAOV1Request;
use crate::deployment::{
    test_address, DeployedScriptInfo, DeployedValidator, DeployedValidatorErased, RequiresValidator,
};
use bloom_offchain::execution_engine::liquidity_book::types::Lovelace;
use cml_chain::plutus::utils::ConstrPlutusDataEncoding;
use cml_chain::plutus::{ConstrPlutusData, PlutusData};
use cml_chain::transaction::TransactionOutput;
use cml_chain::utils::BigInteger;
use cml_core::serialization::LenEncoding::Indefinite;
use cml_core::serialization::RawBytesEncoding;
use cml_crypto::{Ed25519KeyHash, Ed25519Signature, ScriptHash};
use num_rational::Ratio;
use spectrum_cardano_lib::address::{InlineCredential, PlutusAddress};
use spectrum_cardano_lib::plutus_data::{
    ConstrPlutusDataExtension, DatumExtension, IntoPlutusData, PlutusDataExtension,
};
use spectrum_cardano_lib::transaction::TransactionOutputExtension;
use spectrum_cardano_lib::types::TryFromPData;
use spectrum_cardano_lib::{OutputRef, TaggedAmount, TaggedAssetClass};
use spectrum_offchain::data::small_vec::SmallVec;
use spectrum_offchain::domain::Has;
use spectrum_offchain::ledger::TryFromLedger;
use std::option::Option;
use strum_macros::FromRepr;

#[derive(Copy, Clone, Debug)]
pub struct DAOContext {
    pub public_keys: SmallVec<[u8; 32]>,
    pub signature_threshold: u64,
    // Temporal solution. Comes from CML issue of incorrect fee calculation
    // todo: remove after issue resolution
    pub execution_fee: u64,
}

#[derive(FromRepr, Debug, Clone, Eq, PartialEq, Copy)]
#[repr(u64)]
pub enum DaoAction {
    WithdrawTreasury,
    ChangeStakePart,
    ChangeTreasuryFee,
    ChangeTreasuryAddress,
    ChangeAdminAddress,
    ChangePoolFee,
}

impl TryFromPData for DaoAction {
    fn try_from_pd(data: PlutusData) -> Option<Self> {
        if let Some(action_idx) = data.into_u64() {
            return DaoAction::from_repr(action_idx);
        };
        return None;
    }
}

impl IntoPlutusData for DaoAction {
    fn into_pd(self) -> PlutusData {
        PlutusData::Integer(BigInteger::from(self as u8))
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct DAORequestConfig {
    pub dao_action: DaoAction,
    pub pool_nft: TaggedAssetClass<PoolNft>,
    pub pool_fee: u64,
    pub treasury_fee: u64,
    pub admin_address: ScriptHash,
    pub pool_stake_script_hash: Option<ScriptHash>,
    pub treasury_address: ScriptHash,
    pub treasury_x_abs_delta: u64,
    pub treasury_y_abs_delta: u64,
    pub requestor_pkh: Ed25519KeyHash,
    pub signatures: Vec<Ed25519Signature>,
    pub additional_bytes: Vec<Vec<u8>>,
    pub fee: u64,
}

struct DatumMapping {
    pub dao_action: usize,
    pub pool_nft: usize,
    pub pool_fee: usize,
    pub treasury_fee: usize,
    pub admin_address: usize,
    pub pool_address: usize,
    pub treasury_address: usize,
    pub treasury_x_withdraw: usize,
    pub treasury_y_withdraw: usize,
    pub requestor_pkh: usize,
    pub signatures: usize,
    pub additional_bytes: usize,
    pub fee: usize,
}

const DATUM_MAPPING: DatumMapping = DatumMapping {
    dao_action: 0,
    pool_nft: 1,
    pool_fee: 2,
    treasury_fee: 3,
    admin_address: 4,
    pool_address: 5,
    treasury_address: 6,
    treasury_x_withdraw: 7,
    treasury_y_withdraw: 8,
    requestor_pkh: 9,
    signatures: 10,
    additional_bytes: 11,
    fee: 12,
};

impl TryFromPData for DAORequestConfig {
    fn try_from_pd(data: PlutusData) -> Option<Self> {
        let mut cpd = data.into_constr_pd()?;

        let admin_addresses = cpd
            .take_field(DATUM_MAPPING.admin_address)
            .and_then(TryFromPData::try_from_pd)
            .and_then(|creds: Vec<InlineCredential>| {
                creds.first().and_then(|cred| cred.clone().script_hash())
            })?;

        let pool_stake_cred = cpd
            .take_field(DATUM_MAPPING.pool_address)
            .and_then(PlutusAddress::try_from_pd)?
            .stake_cred
            .and_then(|addr| addr.script_hash());

        let treasury_address: ScriptHash = cpd
            .take_field(DATUM_MAPPING.treasury_address)
            .and_then(|pd| pd.into_bytes())
            .and_then(|bytes| ScriptHash::from_raw_bytes(&*bytes).ok())?;

        let requestor_pkh: Ed25519KeyHash = cpd
            .take_field(DATUM_MAPPING.requestor_pkh)
            .and_then(|pd| pd.into_bytes())
            .and_then(|bytes| Some(Ed25519KeyHash::from_raw_bytes(&bytes).ok()?))?;

        let signatures = cpd.take_field(DATUM_MAPPING.signatures).and_then(|pd| {
            pd.into_vec_pd(|raw_signature_data| {
                raw_signature_data
                    .into_bytes()
                    .and_then(|raw_signature| Ed25519Signature::from_raw_bytes(&raw_signature).ok())
            })
        })?;

        let additional_bytes = cpd
            .take_field(DATUM_MAPPING.additional_bytes)
            .and_then(|pd| pd.into_vec_pd(|raw_signature_data| raw_signature_data.into_bytes()))?;

        Some(DAORequestConfig {
            dao_action: DaoAction::try_from_pd(cpd.take_field(DATUM_MAPPING.dao_action)?)?,
            pool_nft: TaggedAssetClass::try_from_pd(cpd.take_field(DATUM_MAPPING.pool_nft)?)?,
            pool_fee: cpd.take_field(DATUM_MAPPING.pool_fee)?.into_u64()?,
            treasury_fee: cpd.take_field(DATUM_MAPPING.treasury_fee)?.into_u64()?,
            admin_address: admin_addresses.clone(),
            pool_stake_script_hash: pool_stake_cred,
            treasury_address,
            treasury_x_abs_delta: cpd.take_field(DATUM_MAPPING.treasury_x_withdraw)?.into_u64()?,
            treasury_y_abs_delta: cpd.take_field(DATUM_MAPPING.treasury_y_withdraw)?.into_u64()?,
            requestor_pkh,
            signatures,
            additional_bytes,
            fee: cpd.take_field(DATUM_MAPPING.fee)?.into_u64()?,
        })
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct DAOV1Request {
    pub dao_action: DaoAction,
    pub pool_nft: PoolId,
    pub pool_fee: Ratio<u64>,
    pub treasury_fee: Ratio<u64>,
    pub admin_address: ScriptHash,
    pub pool_stake_script_hash: Option<ScriptHash>,
    pub treasury_address: ScriptHash,
    pub treasury_x_abs_delta: TaggedAmount<Rx>,
    pub treasury_y_abs_delta: TaggedAmount<Ry>,
    pub requestor_pkh: Ed25519KeyHash,
    pub signatures: Vec<Ed25519Signature>,
    pub additional_bytes: Vec<Vec<u8>>,
    pub fee: Lovelace,
    pub lovelace: Lovelace,
}

#[derive(Copy, Clone, Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DAOV1ActionOrderValidation {
    pub min_collateral_ada: u64,
}

pub type OnChainDAOActionRequest = ClassicalOrder<OnChainOrderId, DAOV1Request>;

impl Into<CFMMPoolAction> for OnChainDAOActionRequest {
    fn into(self) -> CFMMPoolAction {
        DAOAction
    }
}

impl<Ctx> RequiresValidator<Ctx> for OnChainDAOActionRequest
where
    Ctx: Has<DeployedValidator<{ RoyaltyPoolDAOV1Request as u8 }>>,
{
    fn get_validator(&self, ctx: &Ctx) -> DeployedValidatorErased {
        ctx.get().erased()
    }
}

impl<Ctx> TryFromLedger<TransactionOutput, Ctx> for OnChainDAOActionRequest
where
    Ctx: Has<OutputRef>
        + Has<DeployedScriptInfo<{ RoyaltyPoolDAOV1Request as u8 }>>
        + Has<DAOV1ActionOrderValidation>,
{
    fn try_from_ledger(repr: &TransactionOutput, ctx: &Ctx) -> Option<Self> {
        if test_address(repr.address(), ctx) {
            let pd = repr.datum().clone()?.into_pd()?;
            let conf = DAORequestConfig::try_from_pd(pd)?;
            let init_ada_value = repr.value().coin;
            if init_ada_value < conf.fee {
                return None;
            }
            let royalty_withdraw = DAOV1Request {
                dao_action: conf.dao_action,
                pool_nft: PoolId::try_from(conf.pool_nft).ok()?,
                pool_fee: Ratio::new_raw(conf.pool_fee, FEE_DEN),
                treasury_fee: Ratio::new_raw(conf.treasury_fee, FEE_DEN),
                admin_address: conf.admin_address,
                pool_stake_script_hash: conf.pool_stake_script_hash,
                treasury_address: conf.treasury_address,
                treasury_x_abs_delta: TaggedAmount::new(conf.treasury_x_abs_delta),
                treasury_y_abs_delta: TaggedAmount::new(conf.treasury_y_abs_delta),
                requestor_pkh: conf.requestor_pkh,
                signatures: conf.signatures,
                additional_bytes: conf.additional_bytes,
                fee: conf.fee,
                lovelace: init_ada_value,
            };

            let bounds = ctx.select::<DAOV1ActionOrderValidation>();

            if init_ada_value - conf.fee >= bounds.min_collateral_ada {
                return Some(Self {
                    id: OnChainOrderId::from(ctx.select::<OutputRef>()),
                    pool_id: royalty_withdraw.pool_nft,
                    order: royalty_withdraw,
                });
            }
        };
        None
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct DaoRequestDataToSign {
    pub dao_action: DaoAction,
    pub pool_nft: PoolId,
    pub pool_fee: u64,
    pub treasury_fee: u64,
    pub admin_address: Vec<InlineCredential>,
    pub pool_address: PlutusAddress,
    pub treasury_address: ScriptHash,
    // always negative
    pub treasury_x_delta: i64,
    // always negative
    pub treasury_y_delta: i64,
    pub pool_nonce: u64,
}

impl IntoPlutusData for DaoRequestDataToSign {
    fn into_pd(self) -> PlutusData {
        PlutusData::ConstrPlutusData(ConstrPlutusData {
            alternative: 0,
            fields: vec![
                self.dao_action.into_pd(),
                self.pool_nft.0.into_pd(),
                self.pool_fee.into_pd(),
                self.treasury_fee.into_pd(),
                self.admin_address.into_pd(),
                self.pool_address.into_pd(),
                PlutusData::new_bytes(self.treasury_address.to_raw_bytes().to_vec()),
                self.treasury_x_delta.into_pd(),
                self.treasury_y_delta.into_pd(),
                self.pool_nonce.into_pd(),
            ],
            encodings: Some(ConstrPlutusDataEncoding {
                len_encoding: Indefinite,
                tag_encoding: Some(cbor_event::Sz::Inline),
                alternative_encoding: None,
                fields_encoding: Indefinite,
                prefer_compact: true,
            }),
        })
    }
}

#[cfg(test)]
mod tests {
    use crate::data::dao_request::{DAOV1ActionOrderValidation, OnChainDAOActionRequest};
    use crate::data::pool::PoolValidation;
    use crate::deployment::ProtocolValidator::RoyaltyPoolDAOV1Request;
    use crate::deployment::{DeployedScriptInfo, DeployedValidators, ProtocolScriptHashes};
    use cml_chain::transaction::TransactionOutput;
    use cml_core::serialization::Deserialize;
    use cml_crypto::TransactionHash;
    use spectrum_cardano_lib::OutputRef;
    use spectrum_offchain::domain::Has;
    use spectrum_offchain::ledger::TryFromLedger;
    use type_equalities::IsEqual;

    const ORDER_UTXO: &str = "a300581d70b00c33e4ce8227d055fde7fb96e171d91edd8e868db78c642f6ebce7011a00989680028201d81859010bd8799f02d8799f581c34d9e2ffb87551fa04191ed8e5fb2af78e29d023d48dcf429c1590ea436e6674ff1a000173181927109fd8799fd87a9f581cef8bd8fbb18b4236d8c48c16eb60a5b22f8787a4ec911f14d5ff039affffffd8799fd87a9f581cb09d1ccde30a0c65316c8e5dada41ee4d86ca0d0bb5370542693dafaffd87a80ff581cb09d1ccde30a0c65316c8e5dada41ee4d86ca0d0bb5370542693dafa0000581c26077fda71d72f97c3fb62e258f67cd3ffc7c2e0628cc824b95d45099f5840feba296dda6c67108e70f47d2c7b29a852f2d96da5a22dd7e7fec65e5d2f1081a854620d9cc6cbe9f2d9d6b0f2a4916f696ff5787a1a475f992c3a4e1e03fb02ff1a007f1b10ff";

    struct Ctx {
        bounds: PoolValidation,
        scripts: ProtocolScriptHashes,
        output_ref: OutputRef,
        dao_validation: DAOV1ActionOrderValidation,
    }

    impl Has<DeployedScriptInfo<{ RoyaltyPoolDAOV1Request as u8 }>> for Ctx {
        fn select<U: IsEqual<DeployedScriptInfo<{ RoyaltyPoolDAOV1Request as u8 }>>>(
            &self,
        ) -> DeployedScriptInfo<{ RoyaltyPoolDAOV1Request as u8 }> {
            self.scripts.royalty_pool_dao_request
        }
    }

    impl Has<DAOV1ActionOrderValidation> for Ctx {
        fn select<U: IsEqual<DAOV1ActionOrderValidation>>(&self) -> DAOV1ActionOrderValidation {
            self.dao_validation
        }
    }

    impl Has<OutputRef> for Ctx {
        fn select<U: IsEqual<OutputRef>>(&self) -> OutputRef {
            self.output_ref
        }
    }

    #[test]
    fn try_read_order() {
        let raw_deployment = std::fs::read_to_string("/Users/aleksandr/IdeaProjects/spectrum-offchain-multiplatform/bloom-cardano-agent/resources/preprod.deployment.json").expect("Cannot load deployment file");
        let deployment: DeployedValidators =
            serde_json::from_str(&raw_deployment).expect("Invalid deployment file");
        let scripts = ProtocolScriptHashes::from(&deployment);
        let dao_validation = DAOV1ActionOrderValidation {
            min_collateral_ada: 1_500_000,
        };

        let ctx = Ctx {
            scripts,
            bounds: PoolValidation {
                min_n2t_lovelace: 150_000_000,
                min_t2t_lovelace: 10_000_000,
            },
            output_ref: OutputRef::new(
                TransactionHash::from_hex("4a57040cec9d3a02b54d5a1e164ad45c488836389076d7328e5070392adb94ef")
                    .unwrap(),
                0,
            ),
            dao_validation,
        };
        let bearer = TransactionOutput::from_cbor_bytes(&*hex::decode(ORDER_UTXO).unwrap()).unwrap();
        let order = OnChainDAOActionRequest::try_from_ledger(&bearer, &ctx);

        assert_eq!(order.is_some(), true);
    }
}
