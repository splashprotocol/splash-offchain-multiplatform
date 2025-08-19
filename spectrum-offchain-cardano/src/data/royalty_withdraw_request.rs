use crate::data::order::OrderType::{RoyaltyConstFnV1, RoyaltyConstFnV2};
use crate::data::order::{ClassicalOrder, OrderType};
use crate::data::pool::{CFMMPoolAction, Rx, Ry};
use crate::data::{OnChainOrderId, PoolId};
use crate::deployment::ProtocolValidator::{
    RoyaltyPoolV1RoyaltyWithdrawRequest, RoyaltyPoolV2RoyaltyWithdrawRequest,
};
use crate::deployment::{
    test_address, DeployedScriptInfo, DeployedValidator, DeployedValidatorErased, RequiresValidator,
};
use cbor_event::Sz;
use cml_chain::address::Address;
use cml_chain::plutus::utils::ConstrPlutusDataEncoding;
use cml_chain::plutus::{ConstrPlutusData, PlutusData};
use cml_chain::transaction::TransactionOutput;
use cml_core::serialization::LenEncoding::Indefinite;
use cml_core::serialization::StringEncoding::Definite;
use cml_core::serialization::{RawBytesEncoding, Serialize};
use cml_crypto::{Ed25519KeyHash, Ed25519Signature};
use spectrum_cardano_lib::plutus_data::{
    ConstrPlutusDataExtension, DatumExtension, IntoPlutusData, PlutusDataExtension,
};
use spectrum_cardano_lib::transaction::TransactionOutputExtension;
use spectrum_cardano_lib::types::TryFromPData;
use spectrum_cardano_lib::{AssetClass, OutputRef, TaggedAmount, Token};
use spectrum_offchain::domain::Has;
use spectrum_offchain::ledger::TryFromLedger;

// Temporal solution. Comes from CML issue of incorrect fee calculation
// todo: remove after issue resolution
#[derive(Copy, Clone, Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RoyaltyWithdrawContext {
    pub transaction_fee: u64,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct RoyaltyWithdrawRequestConfig {
    pub pool_nft: PoolId,
    pub withdraw_royalty_x: TaggedAmount<Rx>,
    pub withdraw_royalty_y: TaggedAmount<Ry>,
    pub royalty_pub_key_hash: [u8; 28],
    pub fee: u64,
    pub signature: String,
    pub raw_data_to_sign: Vec<u8>,
    pub additional_bytes: Vec<u8>,
}

pub struct RoyaltyWithdrawDataDatumMapping {
    pub pool_nft: usize,
    pub withdraw_royalty_x: usize,
    pub withdraw_royalty_y: usize,
    pub royalty_pub_key_hash: usize,
    pub fee: usize,
}

pub const ROYALTY_WITHDRAW_DATA_DATUM_MAPPING: RoyaltyWithdrawDataDatumMapping =
    RoyaltyWithdrawDataDatumMapping {
        pool_nft: 0,
        withdraw_royalty_x: 1,
        withdraw_royalty_y: 2,
        royalty_pub_key_hash: 3,
        fee: 4,
    };

pub struct RoyaltyWithdrawDatumMapping {
    pub withdraw_data: usize,
    pub signature: usize,
    pub additional_bytes: usize,
}

pub const ROYALTY_WITHDRAW_DATUM_MAPPING: RoyaltyWithdrawDatumMapping = RoyaltyWithdrawDatumMapping {
    withdraw_data: 0,
    signature: 1,
    additional_bytes: 2,
};

impl TryFromPData for RoyaltyWithdrawRequestConfig {
    fn try_from_pd(data: PlutusData) -> Option<Self> {
        let mut cpd = data.clone().into_constr_pd()?;
        let data_to_sign = data
            .into_constr_pd()?
            .take_field(ROYALTY_WITHDRAW_DATUM_MAPPING.withdraw_data)?
            .to_cbor_bytes();
        let mut royalty_data = cpd
            .take_field(ROYALTY_WITHDRAW_DATUM_MAPPING.withdraw_data)?
            .into_constr_pd()?;
        Some(Self {
            pool_nft: PoolId(
                AssetClass::try_from_pd(
                    royalty_data.take_field(ROYALTY_WITHDRAW_DATA_DATUM_MAPPING.pool_nft)?,
                )?
                .into_token()?,
            ),
            withdraw_royalty_x: TaggedAmount::new(
                royalty_data
                    .take_field(ROYALTY_WITHDRAW_DATA_DATUM_MAPPING.withdraw_royalty_x)?
                    .into_u64()?,
            ),
            withdraw_royalty_y: TaggedAmount::new(
                royalty_data
                    .take_field(ROYALTY_WITHDRAW_DATA_DATUM_MAPPING.withdraw_royalty_y)?
                    .into_u64()?,
            ),
            royalty_pub_key_hash: royalty_data
                .take_field(ROYALTY_WITHDRAW_DATA_DATUM_MAPPING.royalty_pub_key_hash)?
                .into_bytes()?
                .try_into()
                .ok()?,
            fee: royalty_data
                .take_field(ROYALTY_WITHDRAW_DATA_DATUM_MAPPING.fee)?
                .into_u64()?,
            signature: hex::encode(
                cpd.take_field(ROYALTY_WITHDRAW_DATUM_MAPPING.signature)?
                    .into_bytes()?,
            ),
            additional_bytes: cpd
                .take_field(ROYALTY_WITHDRAW_DATUM_MAPPING.additional_bytes)?
                .into_bytes()?,
            raw_data_to_sign: data_to_sign,
        })
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct WithdrawData {
    pub pool_nft: Token,
    pub withdraw_royalty_x: u64,
    pub withdraw_royalty_y: u64,
    pub royalty_pub_key_hash: Ed25519KeyHash,
    pub ex_fee: u64,
}

impl IntoPlutusData for WithdrawData {
    fn into_pd(self) -> PlutusData {
        PlutusData::ConstrPlutusData(ConstrPlutusData {
            alternative: 0,
            fields: vec![
                self.pool_nft.into_pd(),
                self.withdraw_royalty_x.into_pd(),
                self.withdraw_royalty_y.into_pd(),
                PlutusData::Bytes {
                    bytes: hex::decode(self.royalty_pub_key_hash.to_raw_hex()).unwrap(),
                    bytes_encoding: Definite(Sz::Inline),
                },
                self.ex_fee.into_pd(),
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

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct DataToSign {
    pub withdraw_data: WithdrawData,
    pub pool_nonce: u64,
}

impl IntoPlutusData for DataToSign {
    fn into_pd(self) -> PlutusData {
        PlutusData::ConstrPlutusData(ConstrPlutusData {
            alternative: 0,
            fields: vec![self.withdraw_data.into_pd(), self.pool_nonce.into_pd()],
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

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct RoyaltyWithdraw {
    pub pool_nft: PoolId,
    pub withdraw_royalty_x: TaggedAmount<Rx>,
    pub withdraw_royalty_y: TaggedAmount<Ry>,
    pub royalty_pub_key_hash: Ed25519KeyHash,
    pub fee: u64,
    pub signature: Ed25519Signature,
    pub init_ada_value: u64,
    pub raw_data_to_sign: Vec<u8>,
    pub additional_bytes: Vec<u8>,
    pub requestor_address: Address,
    pub order_type: OrderType,
}

impl Into<WithdrawData> for RoyaltyWithdraw {
    fn into(self) -> WithdrawData {
        WithdrawData {
            pool_nft: self.pool_nft.0,
            withdraw_royalty_x: self.withdraw_royalty_x.untag(),
            withdraw_royalty_y: self.withdraw_royalty_y.untag(),
            royalty_pub_key_hash: self.royalty_pub_key_hash,
            ex_fee: self.fee,
        }
    }
}

#[derive(Copy, Clone, Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RoyaltyWithdrawOrderValidation {
    pub min_ada_in_royalty_output: u64,
}

pub type OnChainRoyaltyWithdraw = ClassicalOrder<OnChainOrderId, RoyaltyWithdraw>;

impl<Ctx> RequiresValidator<Ctx> for OnChainRoyaltyWithdraw
where
    Ctx: Has<DeployedValidator<{ RoyaltyPoolV1RoyaltyWithdrawRequest as u8 }>>
        + Has<DeployedValidator<{ RoyaltyPoolV2RoyaltyWithdrawRequest as u8 }>>,
{
    fn get_validator(&self, ctx: &Ctx) -> DeployedValidatorErased {
        match self.order.order_type {
            RoyaltyConstFnV1 => {
                let validator: DeployedValidator<{ RoyaltyPoolV1RoyaltyWithdrawRequest as u8 }> = ctx.get();
                validator.erased()
            }
            RoyaltyConstFnV2 => {
                let validator: DeployedValidator<{ RoyaltyPoolV2RoyaltyWithdrawRequest as u8 }> = ctx.get();
                validator.erased()
            }
            _ => unreachable!(),
        }
    }
}

impl Into<CFMMPoolAction> for OnChainRoyaltyWithdraw {
    fn into(self) -> CFMMPoolAction {
        CFMMPoolAction::RoyaltyWithdraw
    }
}

impl<Ctx> TryFromLedger<TransactionOutput, Ctx> for OnChainRoyaltyWithdraw
where
    Ctx: Has<OutputRef>
        + Has<DeployedScriptInfo<{ RoyaltyPoolV1RoyaltyWithdrawRequest as u8 }>>
        + Has<DeployedScriptInfo<{ RoyaltyPoolV2RoyaltyWithdrawRequest as u8 }>>
        + Has<RoyaltyWithdrawOrderValidation>,
{
    fn try_from_ledger(repr: &TransactionOutput, ctx: &Ctx) -> Option<Self> {
        let is_royalty_v1_fn_withdraw =
            test_address::<{ RoyaltyPoolV1RoyaltyWithdrawRequest as u8 }, Ctx>(repr.address(), ctx);
        let is_royalty_v2_fn_withdraw =
            test_address::<{ RoyaltyPoolV2RoyaltyWithdrawRequest as u8 }, Ctx>(repr.address(), ctx);

        if is_royalty_v1_fn_withdraw || is_royalty_v2_fn_withdraw {
            let pd = repr.datum().clone()?.into_pd()?;
            let conf = RoyaltyWithdrawRequestConfig::try_from_pd(pd)?;
            let init_ada_value = repr.value().coin;
            let order_type = if is_royalty_v1_fn_withdraw {
                RoyaltyConstFnV1
            } else {
                RoyaltyConstFnV2
            };
            let royalty_withdraw = RoyaltyWithdraw {
                pool_nft: conf.pool_nft,
                withdraw_royalty_x: conf.withdraw_royalty_x,
                withdraw_royalty_y: conf.withdraw_royalty_y,
                royalty_pub_key_hash: Ed25519KeyHash::from(conf.royalty_pub_key_hash),
                fee: conf.fee,
                signature: Ed25519Signature::from_raw_hex(conf.signature.as_str()).ok()?,
                init_ada_value,
                raw_data_to_sign: conf.raw_data_to_sign,
                additional_bytes: conf.additional_bytes,
                requestor_address: repr.address().clone(),
                order_type,
            };
            let bounds = ctx.select::<RoyaltyWithdrawOrderValidation>();

            if init_ada_value - conf.fee >= bounds.min_ada_in_royalty_output {
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
