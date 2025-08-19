use crate::constants::{FEE_DEN, MAX_LQ_CAP, POOL_OUT_IDX_IN};
use crate::data::cfmm_pool::AMMOps;
use crate::data::dao_request::{DAOContext, DaoAction, DaoRequestDataToSign, OnChainDAOActionRequest};
use crate::data::operation_output::DaoActionResult::{RequestorOutput, TreasuryWithdraw};
use crate::data::operation_output::OperationResultOutputs::SingleOutput;
use crate::data::operation_output::{
    ContexBasedRedeemerCreator, DaoActionResult, DaoRequestorOutput, OperationResultBlueprint,
    OperationResultContext, OperationResultOutputs, RoyaltyWithdrawOutput, TreasuryWithdrawOutput,
};
use crate::data::order::{Base, PoolNft, Quote};
use crate::data::pair::order_canonical;
use crate::data::pool::{ApplyOrder, ApplyOrderError, ImmutablePoolUtxo, Lq, PoolValidation, Rx, Ry};
use crate::data::royalty_withdraw_request::{DataToSign, OnChainRoyaltyWithdraw, RoyaltyWithdrawContext};
use crate::data::PoolId;
use crate::deployment::ProtocolValidator::{
    RoyaltyPoolDAOV1, RoyaltyPoolDAOV1Request, RoyaltyPoolRoyaltyWithdraw,
    RoyaltyPoolRoyaltyWithdrawLedgerFixed, RoyaltyPoolRoyaltyWithdrawV2, RoyaltyPoolV1,
    RoyaltyPoolV1LedgerFixed, RoyaltyPoolV1RoyaltyWithdrawRequest, RoyaltyPoolV2, RoyaltyPoolV2DAO,
    RoyaltyPoolV2RoyaltyWithdrawRequest,
};
use crate::deployment::{DeployedScriptInfo, DeployedValidator, DeployedValidatorErased, RequiresValidator};
use crate::pool_math::cfmm_math::{
    classic_cfmm_output_amount, classic_cfmm_reward_lp, classic_cfmm_shares_amount,
    UNTOUCHABLE_LOVELACE_AMOUNT,
};
use bignumber::BigNumber;
use bloom_offchain::execution_engine::liquidity_book::core::Next;
use bloom_offchain::execution_engine::liquidity_book::market_maker::{
    AbsoluteReserves, AvailableLiquidity, MakerBehavior, MarketMaker, PoolQuality, SpotPrice,
};
use bloom_offchain::execution_engine::liquidity_book::side::OnSide;
use bloom_offchain::execution_engine::liquidity_book::types::AbsolutePrice;
use cml_chain::address::Address::Enterprise;
use cml_chain::address::{Address, EnterpriseAddress};
use cml_chain::assets::MultiAsset;
use cml_chain::certs::{Credential, StakeCredential};
use cml_chain::plutus::{ConstrPlutusData, PlutusData};
use cml_chain::transaction::{ConwayFormatTxOut, DatumOption, TransactionOutput};
use cml_chain::utils::BigInteger;
use cml_chain::Value;
use cml_core::serialization::{RawBytesEncoding, Serialize};
use cml_crypto::{blake2b224, Ed25519Signature, PublicKey, ScriptHash};
use num_rational::Ratio;
use num_traits::{CheckedSub, ToPrimitive};
use spectrum_cardano_lib::address::{AddressExtension, InlineCredential, PlutusAddress, PlutusCredential};
use spectrum_cardano_lib::ex_units::ExUnits;
use spectrum_cardano_lib::plutus_data::{ConstrPlutusDataExtension, DatumExtension};
use spectrum_cardano_lib::plutus_data::{IntoPlutusData, PlutusDataExtension};
use spectrum_cardano_lib::transaction::TransactionOutputExtension;
use spectrum_cardano_lib::types::TryFromPData;
use spectrum_cardano_lib::value::ValueExtension;
use spectrum_cardano_lib::AssetClass::Native;
use spectrum_cardano_lib::{Ed25519PublicKey, NetworkId, TaggedAmount, TaggedAssetClass, Token};
use spectrum_offchain::domain::{Has, Stable};
use spectrum_offchain::ledger::{IntoLedger, TryFromLedger};
use std::fmt::{Debug, Display, Formatter};
use std::ops::{Div, Neg};
use void::Void;

pub const AIKEN_TRUE: PlutusData = PlutusData::ConstrPlutusData(ConstrPlutusData {
    alternative: 1,
    fields: vec![],
    encodings: None,
});

pub const AIKEN_FALSE: PlutusData = PlutusData::ConstrPlutusData(ConstrPlutusData {
    alternative: 0,
    fields: vec![],
    encodings: None,
});

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

pub fn unsafe_update_pd_royalty(
    data: &mut PlutusData,
    lp_fee_num: u64,
    treasury_fee_num: u64,
    royalty_fee_num: u64,
    treasury_x: u64,
    treasury_y: u64,
    royalty_x: u64,
    royalty_y: u64,
    treasury_address: ScriptHash,
    admin_address: ScriptHash,
    new_nonce: u64,
) {
    let admin_addresses: Vec<InlineCredential> = vec![admin_address.into()];

    let cpd = data.get_constr_pd_mut().unwrap();
    cpd.set_field(ROYALTY_DATUM_MAPPING.lp_fee_num, lp_fee_num.into_pd());
    cpd.set_field(ROYALTY_DATUM_MAPPING.treasury_fee_num, treasury_fee_num.into_pd());
    cpd.set_field(ROYALTY_DATUM_MAPPING.royalty_fee_num, royalty_fee_num.into_pd());
    cpd.set_field(ROYALTY_DATUM_MAPPING.treasury_x, treasury_x.into_pd());
    cpd.set_field(ROYALTY_DATUM_MAPPING.treasury_y, treasury_y.into_pd());
    cpd.set_field(ROYALTY_DATUM_MAPPING.royalty_x, royalty_x.into_pd());
    cpd.set_field(ROYALTY_DATUM_MAPPING.royalty_y, royalty_y.into_pd());
    cpd.set_field(ROYALTY_DATUM_MAPPING.admin_address, admin_addresses.into_pd());
    cpd.set_field(
        ROYALTY_DATUM_MAPPING.treasury_address,
        treasury_address.to_raw_bytes().to_vec().into_pd(),
    );
    cpd.set_field(ROYALTY_DATUM_MAPPING.royalty_nonce, new_nonce.into_pd());
}

pub fn unsafe_update_pd_royalty_v2(
    data: &mut PlutusData,
    lp_fee_num: u64,
    treasury_fee_num: u64,
    first_royalty_fee_num: u64,
    second_royalty_fee_num: u64,
    treasury_x: u64,
    treasury_y: u64,
    first_royalty_x: u64,
    first_royalty_y: u64,
    second_royalty_x: u64,
    second_royalty_y: u64,
    treasury_address: ScriptHash,
    admin_address: ScriptHash,
    new_nonce: u64,
) {
    let admin_addresses: Vec<InlineCredential> = vec![admin_address.into()];

    let cpd = data.get_constr_pd_mut().unwrap();
    cpd.set_field(ROYALTY_V2_DATUM_MAPPING.lp_fee_num, lp_fee_num.into_pd());
    cpd.set_field(
        ROYALTY_V2_DATUM_MAPPING.treasury_fee_num,
        treasury_fee_num.into_pd(),
    );
    cpd.set_field(
        ROYALTY_V2_DATUM_MAPPING.first_royalty_fee_num,
        first_royalty_fee_num.into_pd(),
    );
    cpd.set_field(
        ROYALTY_V2_DATUM_MAPPING.second_royalty_fee_num,
        second_royalty_fee_num.into_pd(),
    );
    cpd.set_field(ROYALTY_V2_DATUM_MAPPING.treasury_x, treasury_x.into_pd());
    cpd.set_field(ROYALTY_V2_DATUM_MAPPING.treasury_y, treasury_y.into_pd());
    cpd.set_field(
        ROYALTY_V2_DATUM_MAPPING.first_royalty_x,
        first_royalty_x.into_pd(),
    );
    cpd.set_field(
        ROYALTY_V2_DATUM_MAPPING.first_royalty_y,
        first_royalty_y.into_pd(),
    );
    cpd.set_field(
        ROYALTY_V2_DATUM_MAPPING.second_royalty_x,
        second_royalty_x.into_pd(),
    );
    cpd.set_field(
        ROYALTY_V2_DATUM_MAPPING.second_royalty_y,
        second_royalty_y.into_pd(),
    );
    cpd.set_field(ROYALTY_V2_DATUM_MAPPING.admin_address, admin_addresses.into_pd());
    cpd.set_field(
        ROYALTY_V2_DATUM_MAPPING.treasury_address,
        treasury_address.to_raw_bytes().to_vec().into_pd(),
    );
    cpd.set_field(ROYALTY_V2_DATUM_MAPPING.royalty_nonce, new_nonce.into_pd());
}

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

#[derive(Debug)]
pub struct RoyaltyPoolV2Config {
    pub pool_nft: TaggedAssetClass<PoolNft>,
    pub asset_x: TaggedAssetClass<Rx>,
    pub asset_y: TaggedAssetClass<Ry>,
    pub asset_lq: TaggedAssetClass<Lq>,
    pub lp_fee_num: u64,
    pub treasury_fee_num: u64,
    pub first_royalty_fee_num: u64,
    pub second_royalty_fee_num: u64,
    pub treasury_x: u64,
    pub treasury_y: u64,
    pub first_royalty_x: u64,
    pub first_royalty_y: u64,
    pub second_royalty_x: u64,
    pub second_royalty_y: u64,
    pub admin_address: ScriptHash,
    pub treasury_address: Vec<u8>,
    pub first_royalty_pub_key: Vec<u8>,
    pub second_royalty_pub_key: Vec<u8>,
    pub nonce: u64,
}

pub struct RoyaltyPoolV2DatumMapping {
    pub pool_nft: usize,
    pub asset_x: usize,
    pub asset_y: usize,
    pub asset_lq: usize,
    pub lp_fee_num: usize,
    pub treasury_fee_num: usize,
    pub first_royalty_fee_num: usize,
    pub second_royalty_fee_num: usize,
    pub treasury_x: usize,
    pub treasury_y: usize,
    pub first_royalty_x: usize,
    pub first_royalty_y: usize,
    pub second_royalty_x: usize,
    pub second_royalty_y: usize,
    pub admin_address: usize,
    pub treasury_address: usize,
    pub first_royalty_pub_key: usize,
    pub second_royalty_pub_key: usize,
    pub royalty_nonce: usize,
}

pub const ROYALTY_V2_DATUM_MAPPING: RoyaltyPoolV2DatumMapping = RoyaltyPoolV2DatumMapping {
    pool_nft: 0,
    asset_x: 1,
    asset_y: 2,
    asset_lq: 3,
    lp_fee_num: 4,
    treasury_fee_num: 5,
    first_royalty_fee_num: 6,
    second_royalty_fee_num: 7,
    treasury_x: 8,
    treasury_y: 9,
    first_royalty_x: 10,
    first_royalty_y: 11,
    second_royalty_x: 12,
    second_royalty_y: 13,
    admin_address: 14,
    treasury_address: 15,
    first_royalty_pub_key: 16,
    second_royalty_pub_key: 17,
    royalty_nonce: 18,
};

impl TryFromPData for RoyaltyPoolV2Config {
    fn try_from_pd(data: PlutusData) -> Option<Self> {
        let mut cpd = data.into_constr_pd()?;

        let admin_address = cpd
            .clone()
            .take_field(ROYALTY_V2_DATUM_MAPPING.admin_address)
            .and_then(TryFromPData::try_from_pd)
            .and_then(|creds: Vec<InlineCredential>| {
                creds.first().and_then(|cred| cred.clone().script_hash())
            })?;

        Some(Self {
            pool_nft: TaggedAssetClass::try_from_pd(cpd.take_field(ROYALTY_V2_DATUM_MAPPING.pool_nft)?)?,
            asset_x: TaggedAssetClass::try_from_pd(cpd.take_field(ROYALTY_V2_DATUM_MAPPING.asset_x)?)?,
            asset_y: TaggedAssetClass::try_from_pd(cpd.take_field(ROYALTY_V2_DATUM_MAPPING.asset_y)?)?,
            asset_lq: TaggedAssetClass::try_from_pd(cpd.take_field(ROYALTY_V2_DATUM_MAPPING.asset_lq)?)?,
            lp_fee_num: cpd.take_field(ROYALTY_V2_DATUM_MAPPING.lp_fee_num)?.into_u64()?,
            treasury_fee_num: cpd
                .take_field(ROYALTY_V2_DATUM_MAPPING.treasury_fee_num)?
                .into_u64()?,
            first_royalty_fee_num: cpd
                .take_field(ROYALTY_V2_DATUM_MAPPING.first_royalty_fee_num)?
                .into_u64()?,
            second_royalty_fee_num: cpd
                .take_field(ROYALTY_V2_DATUM_MAPPING.second_royalty_fee_num)?
                .into_u64()?,
            treasury_x: cpd.take_field(ROYALTY_V2_DATUM_MAPPING.treasury_x)?.into_u64()?,
            treasury_y: cpd.take_field(ROYALTY_V2_DATUM_MAPPING.treasury_y)?.into_u64()?,
            first_royalty_x: cpd
                .take_field(ROYALTY_V2_DATUM_MAPPING.first_royalty_x)?
                .into_u64()?,
            first_royalty_y: cpd
                .take_field(ROYALTY_V2_DATUM_MAPPING.first_royalty_y)?
                .into_u64()?,
            second_royalty_x: cpd
                .take_field(ROYALTY_V2_DATUM_MAPPING.second_royalty_x)?
                .into_u64()?,
            second_royalty_y: cpd
                .take_field(ROYALTY_V2_DATUM_MAPPING.second_royalty_y)?
                .into_u64()?,
            admin_address: admin_address,
            treasury_address: cpd
                .take_field(ROYALTY_V2_DATUM_MAPPING.treasury_address)?
                .into_bytes()?,
            first_royalty_pub_key: cpd
                .take_field(ROYALTY_V2_DATUM_MAPPING.first_royalty_pub_key)?
                .into_bytes()?,
            second_royalty_pub_key: cpd
                .take_field(ROYALTY_V2_DATUM_MAPPING.second_royalty_pub_key)?
                .into_bytes()?,
            nonce: cpd
                .take_field(ROYALTY_V2_DATUM_MAPPING.royalty_nonce)?
                .into_u64()?,
        })
    }
}

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub enum RoyaltyPoolVer {
    V1,
    V1LedgerFixed,
    V2,
}

impl RoyaltyPoolVer {
    pub fn try_from_address<Ctx>(pool_addr: &Address, ctx: &Ctx) -> Option<RoyaltyPoolVer>
    where
        Ctx: Has<DeployedScriptInfo<{ RoyaltyPoolV1 as u8 }>>
            + Has<DeployedScriptInfo<{ RoyaltyPoolV1LedgerFixed as u8 }>>
            + Has<DeployedScriptInfo<{ RoyaltyPoolV2 as u8 }>>,
    {
        let maybe_hash = pool_addr.payment_cred().and_then(|c| match c {
            StakeCredential::PubKey { .. } => None,
            StakeCredential::Script { hash, .. } => Some(hash),
        });

        if let Some(this_hash) = maybe_hash {
            if ctx
                .select::<DeployedScriptInfo<{ RoyaltyPoolV1 as u8 }>>()
                .script_hash
                == *this_hash
            {
                return Some(RoyaltyPoolVer::V1);
            } else if ctx
                .select::<DeployedScriptInfo<{ RoyaltyPoolV1LedgerFixed as u8 }>>()
                .script_hash
                == *this_hash
            {
                return Some(RoyaltyPoolVer::V1LedgerFixed);
            } else if ctx
                .select::<DeployedScriptInfo<{ RoyaltyPoolV2 as u8 }>>()
                .script_hash
                == *this_hash
            {
                return Some(RoyaltyPoolVer::V2);
            }
        };
        None
    }
}

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub struct RoyaltyPool {
    pub id: PoolId,
    pub reserves_x: TaggedAmount<Rx>,
    pub reserves_y: TaggedAmount<Ry>,
    pub liquidity: TaggedAmount<Lq>,
    pub asset_x: TaggedAssetClass<Rx>,
    pub asset_y: TaggedAssetClass<Ry>,
    pub asset_lq: TaggedAssetClass<Lq>,
    pub lp_fee: Ratio<u64>,
    pub treasury_fee: Ratio<u64>,
    pub treasury_x: TaggedAmount<Rx>,
    pub treasury_y: TaggedAmount<Ry>,
    pub first_royalty_fee: Ratio<u64>,
    pub second_royalty_fee: Ratio<u64>,
    pub first_royalty_x: TaggedAmount<Rx>,
    pub first_royalty_y: TaggedAmount<Ry>,
    pub second_royalty_x: TaggedAmount<Rx>,
    pub second_royalty_y: TaggedAmount<Ry>,
    pub lq_lower_bound: TaggedAmount<Rx>,
    pub admin_address: ScriptHash,
    pub treasury_address: ScriptHash,
    pub first_royalty_pub_key: Ed25519PublicKey,
    pub second_royalty_pub_key: Ed25519PublicKey,
    pub ver: RoyaltyPoolVer,
    pub marginal_cost: ExUnits,
    pub bounds: PoolValidation,
    pub nonce: u64,
    pub stake_part_script_hash: Option<ScriptHash>,
}

impl Display for RoyaltyPool {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self.ver {
            RoyaltyPoolVer::V1 | RoyaltyPoolVer::V1LedgerFixed => {
                f.write_str(&*format!(
                    "RoyaltyPool(id: {}, ver: V1, static_price: {}, rx: {}, ry: {},  tx: {}, ty: {}, royalty_x: {}, royalty_y: {})",
                    self.id,
                    self.static_price(),
                    self.reserves_x,
                    self.reserves_y,
                    self.treasury_x,
                    self.treasury_y,
                    self.first_royalty_x,
                    self.first_royalty_y,
                ))
            }
            RoyaltyPoolVer::V2 => {
                f.write_str(&*format!(
                    "RoyaltyPool(id: {}, ver: V2, static_price: {}, rx: {}, ry: {},  tx: {}, ty: {}, first_royalty_x: {}, first_royalty_y: {}, second_royalty_x: {}, second_royalty_y: {})",
                    self.id,
                    self.static_price(),
                    self.reserves_x,
                    self.reserves_y,
                    self.treasury_x,
                    self.treasury_y,
                    self.first_royalty_x,
                    self.first_royalty_y,
                    self.second_royalty_x,
                    self.second_royalty_y,
                ))
            }
        }
    }
}

impl<Ctx> TryFromLedger<TransactionOutput, Ctx> for RoyaltyPool
where
    Ctx: Has<DeployedScriptInfo<{ RoyaltyPoolV1 as u8 }>>
        + Has<DeployedScriptInfo<{ RoyaltyPoolV1LedgerFixed as u8 }>>
        + Has<DeployedScriptInfo<{ RoyaltyPoolV2 as u8 }>>
        + Has<PoolValidation>,
{
    fn try_from_ledger(repr: &TransactionOutput, ctx: &Ctx) -> Option<Self> {
        if let Some(pool_ver) = RoyaltyPoolVer::try_from_address(repr.address(), ctx) {
            let value = repr.value();
            let pd = repr.datum().clone()?.into_pd()?;
            let bounds = ctx.select::<PoolValidation>();
            let stake_part = repr
                .address()
                .staking_cred()
                .and_then(|staking_cred| match staking_cred {
                    StakeCredential::Script { hash, .. } => Some(*hash),
                    _ => None,
                });
            let marginal_cost = match pool_ver {
                RoyaltyPoolVer::V1 => {
                    ctx.select::<DeployedScriptInfo<{ RoyaltyPoolV1 as u8 }>>()
                        .marginal_cost
                }
                RoyaltyPoolVer::V1LedgerFixed => {
                    ctx.select::<DeployedScriptInfo<{ RoyaltyPoolV1LedgerFixed as u8 }>>()
                        .marginal_cost
                }
                RoyaltyPoolVer::V2 => {
                    ctx.select::<DeployedScriptInfo<{ RoyaltyPoolV2 as u8 }>>()
                        .marginal_cost
                }
            };
            match pool_ver {
                RoyaltyPoolVer::V1 | RoyaltyPoolVer::V1LedgerFixed => {
                    let conf = RoyaltyPoolConfig::try_from_pd(pd.clone())?;
                    let liquidity_neg = value.amount_of(conf.asset_lq.into())?;
                    let lov = value.amount_of(Native)?;
                    let reserves_x = value.amount_of(conf.asset_x.into())?;
                    let reserves_y = value.amount_of(conf.asset_y.into())?;
                    let pure_reserves_x = reserves_x - conf.treasury_x - conf.royalty_x;
                    let pure_reserves_y = reserves_y - conf.treasury_y - conf.royalty_y;
                    let non_empty_reserves = pure_reserves_x > 0 && pure_reserves_y > 0;
                    let sufficient_lovelace = conf.asset_x.is_native()
                        || conf.asset_y.is_native()
                        || bounds.min_t2t_lovelace <= lov;
                    let enough_reserves = reserves_x > (conf.treasury_x + conf.royalty_x)
                        && reserves_y > (conf.treasury_y + conf.royalty_y);
                    if non_empty_reserves && sufficient_lovelace && enough_reserves {
                        return Some(RoyaltyPool {
                            id: PoolId::try_from(conf.pool_nft).ok()?,
                            reserves_x: TaggedAmount::new(reserves_x),
                            reserves_y: TaggedAmount::new(reserves_y),
                            liquidity: TaggedAmount::new(MAX_LQ_CAP - liquidity_neg),
                            asset_x: conf.asset_x,
                            asset_y: conf.asset_y,
                            asset_lq: conf.asset_lq,
                            lp_fee: Ratio::new_raw(conf.lp_fee_num, FEE_DEN),
                            treasury_fee: Ratio::new_raw(conf.treasury_fee_num, FEE_DEN),
                            treasury_x: TaggedAmount::new(conf.treasury_x),
                            treasury_y: TaggedAmount::new(conf.treasury_y),
                            first_royalty_fee: Ratio::new_raw(conf.royalty_fee_num, FEE_DEN),
                            second_royalty_fee: Ratio::new_raw(0, 100),
                            first_royalty_x: TaggedAmount::new(conf.royalty_x),
                            first_royalty_y: TaggedAmount::new(conf.royalty_y),
                            second_royalty_x: TaggedAmount::new(0),
                            second_royalty_y: TaggedAmount::new(0),
                            lq_lower_bound: TaggedAmount::new(0),
                            admin_address: conf.admin_address,
                            treasury_address: ScriptHash::from_raw_bytes(&conf.treasury_address).ok()?,
                            first_royalty_pub_key: conf.royalty_pub_key.try_into().ok()?,
                            second_royalty_pub_key: Ed25519PublicKey::from([0; 32]),
                            ver: pool_ver,
                            marginal_cost,
                            bounds,
                            nonce: conf.nonce,
                            stake_part_script_hash: stake_part,
                        });
                    }
                }
                RoyaltyPoolVer::V2 => {
                    let conf = RoyaltyPoolV2Config::try_from_pd(pd.clone())?;
                    let liquidity_neg = value.amount_of(conf.asset_lq.into())?;
                    let lov = value.amount_of(Native)?;
                    let reserves_x = value.amount_of(conf.asset_x.into())?;
                    let reserves_y = value.amount_of(conf.asset_y.into())?;
                    let pure_reserves_x =
                        reserves_x - conf.treasury_x - conf.first_royalty_x - conf.second_royalty_x;
                    let pure_reserves_y =
                        reserves_y - conf.treasury_y - conf.first_royalty_y - conf.second_royalty_y;
                    let non_empty_reserves = pure_reserves_x > 0 && pure_reserves_y > 0;
                    let sufficient_lovelace = conf.asset_x.is_native()
                        || conf.asset_y.is_native()
                        || bounds.min_t2t_lovelace <= lov;
                    let enough_reserves = reserves_x
                        > (conf.treasury_x + conf.first_royalty_x + conf.second_royalty_x)
                        && reserves_y > (conf.treasury_y + conf.first_royalty_y + conf.second_royalty_y);
                    if non_empty_reserves && sufficient_lovelace && enough_reserves {
                        return Some(RoyaltyPool {
                            id: PoolId::try_from(conf.pool_nft).ok()?,
                            reserves_x: TaggedAmount::new(reserves_x),
                            reserves_y: TaggedAmount::new(reserves_y),
                            liquidity: TaggedAmount::new(MAX_LQ_CAP - liquidity_neg),
                            asset_x: conf.asset_x,
                            asset_y: conf.asset_y,
                            asset_lq: conf.asset_lq,
                            lp_fee: Ratio::new_raw(conf.lp_fee_num, FEE_DEN),
                            treasury_fee: Ratio::new_raw(conf.treasury_fee_num, FEE_DEN),
                            treasury_x: TaggedAmount::new(conf.treasury_x),
                            treasury_y: TaggedAmount::new(conf.treasury_y),
                            first_royalty_fee: Ratio::new_raw(conf.first_royalty_fee_num, FEE_DEN),
                            second_royalty_fee: Ratio::new_raw(conf.second_royalty_fee_num, FEE_DEN),
                            first_royalty_x: TaggedAmount::new(conf.first_royalty_x),
                            first_royalty_y: TaggedAmount::new(conf.first_royalty_y),
                            second_royalty_x: TaggedAmount::new(conf.second_royalty_x),
                            second_royalty_y: TaggedAmount::new(conf.second_royalty_y),
                            lq_lower_bound: TaggedAmount::new(0),
                            admin_address: conf.admin_address,
                            treasury_address: ScriptHash::from_raw_bytes(&conf.treasury_address).ok()?,
                            first_royalty_pub_key: conf.first_royalty_pub_key.try_into().ok()?,
                            second_royalty_pub_key: conf.second_royalty_pub_key.try_into().ok()?,
                            ver: pool_ver,
                            marginal_cost,
                            bounds,
                            nonce: conf.nonce,
                            stake_part_script_hash: stake_part,
                        });
                    }
                }
            }
        };
        None
    }
}

impl<Ctx> ApplyOrder<OnChainDAOActionRequest, Ctx> for RoyaltyPool
where
    Ctx: Has<DeployedValidator<{ RoyaltyPoolDAOV1 as u8 }>>
        + Has<DeployedValidator<{ RoyaltyPoolV2DAO as u8 }>>
        + Has<DeployedValidator<{ RoyaltyPoolDAOV1Request as u8 }>>
        + Has<DeployedValidator<{ RoyaltyPoolV1 as u8 }>>
        + Has<DeployedValidator<{ RoyaltyPoolV2 as u8 }>>
        + Has<DAOContext>
        + Has<NetworkId>,
{
    type Result = DaoActionResult;

    fn apply_order(
        mut self,
        dao_request: OnChainDAOActionRequest,
        ctx: Ctx,
    ) -> Result<(Self, OperationResultBlueprint<DaoActionResult>), ApplyOrderError<OnChainDAOActionRequest>>
    {
        let validator = dao_request.get_validator(&ctx);
        let dao_validator = if self.ver == RoyaltyPoolVer::V1 {
            ctx.select::<DeployedValidator<{ RoyaltyPoolDAOV1 as u8 }>>()
                .erased()
        } else {
            ctx.select::<DeployedValidator<{ RoyaltyPoolV2DAO as u8 }>>()
                .erased()
        };

        let pool_validator_hash = if self.ver == RoyaltyPoolVer::V2 {
            ctx.select::<DeployedValidator<{ RoyaltyPoolV1 as u8 }>>().hash
        } else {
            ctx.select::<DeployedValidator<{ RoyaltyPoolV2 as u8 }>>().hash
        };
        let dao_ctx: DAOContext = ctx.get();

        let data_to_sign: DaoRequestDataToSign = DaoRequestDataToSign {
            dao_action: dao_request.order.dao_action,
            pool_nft: dao_request.order.pool_nft,
            pool_fee: *dao_request.order.pool_fee.numer(),
            treasury_fee: *dao_request.order.treasury_fee.numer(),
            admin_address: vec![dao_request.order.admin_address.into()],
            pool_address: PlutusAddress {
                payment_cred: PlutusCredential::Script(pool_validator_hash),
                stake_cred: dao_request.order.pool_stake_script_hash.map(Into::into),
            },
            treasury_address: dao_request.order.treasury_address,
            // on contract side we are validating delta between `new_treasury_x` and
            // `prev_treasury_x`
            treasury_x_delta: (dao_request.order.treasury_x_abs_delta.untag() as i64).neg(),
            treasury_y_delta: (dao_request.order.treasury_y_abs_delta.untag() as i64).neg(),
            pool_nonce: self.nonce,
        };

        let data_to_sign_raw = data_to_sign.clone().into_pd().to_cbor_bytes();

        let data_to_sign_with_additional_bytes: Vec<Vec<u8>> = dao_request
            .clone()
            .order
            .additional_bytes
            .into_iter()
            .map(|mut additional_bytes| {
                let mut to_add = data_to_sign_raw.clone();
                additional_bytes.append(&mut to_add);
                additional_bytes
            })
            .collect();

        let mut siq_qty = 0;

        let correct_signatures_qty = dao_request
            .clone()
            .order
            .signatures
            .into_iter()
            .zip::<Vec<[u8; 32]>>(dao_ctx.clone().public_keys.into())
            .zip(data_to_sign_with_additional_bytes)
            .fold(
                0,
                |correct_signatures, (signature_with_public_key, data_to_sign_for_user)| {
                    siq_qty += 1;
                    let (signature, public_key_raw) = signature_with_public_key;
                    if let Some(public_key) = PublicKey::from_raw_bytes(&public_key_raw).ok() {
                        if public_key.verify(&data_to_sign_for_user, &signature) {
                            return correct_signatures + 1;
                        }
                    }
                    correct_signatures
                },
            );

        if correct_signatures_qty < dao_ctx.signature_threshold {
            return Err(ApplyOrderError::verification_failed(
                dao_request.clone(),
                format!(
                    "Incorrect signature threshold. Correct signatures qty: {}. Threshold: {}",
                    correct_signatures_qty, dao_ctx.signature_threshold
                ),
            ));
        }

        let network_id = ctx.select::<NetworkId>();
        match dao_request.order.dao_action {
            DaoAction::WithdrawTreasury => {
                self.reserves_x = self
                    .reserves_x
                    .checked_sub(&dao_request.order.treasury_x_abs_delta)
                    .ok_or(ApplyOrderError::incompatible(dao_request.clone()))?;
                self.treasury_x = self
                    .treasury_x
                    .checked_sub(&dao_request.order.treasury_x_abs_delta)
                    .ok_or(ApplyOrderError::incompatible(dao_request.clone()))?;
                self.reserves_y = self
                    .reserves_y
                    .checked_sub(&dao_request.order.treasury_y_abs_delta)
                    .ok_or(ApplyOrderError::incompatible(dao_request.clone()))?;
                self.treasury_y = self
                    .treasury_y
                    .checked_sub(&dao_request.order.treasury_y_abs_delta)
                    .ok_or(ApplyOrderError::incompatible(dao_request.clone()))?
            }
            DaoAction::ChangeStakePart => {
                self.stake_part_script_hash = dao_request.order.pool_stake_script_hash
            }
            DaoAction::ChangeTreasuryFee => self.treasury_fee = dao_request.order.treasury_fee,
            DaoAction::ChangeTreasuryAddress => self.treasury_address = dao_request.order.treasury_address,
            DaoAction::ChangeAdminAddress => self.admin_address = dao_request.order.admin_address,
            DaoAction::ChangePoolFee => {
                self.lp_fee = dao_request.order.pool_fee;
            }
        };

        let requestor_output = RequestorOutput(DaoRequestorOutput {
            lovelace_qty: dao_request
                .order
                .lovelace
                .checked_sub(dao_request.order.fee)
                .ok_or(ApplyOrderError::incompatible(dao_request.clone()))?,
            requestor_address: Enterprise(EnterpriseAddress::new(
                network_id.into(),
                Credential::new_pub_key(dao_request.order.requestor_pkh),
            )),
        });

        self.nonce += 1;

        let redeemer_creator = |pool_id: PoolId,
                                signatures: Vec<Ed25519Signature>,
                                additional_bytes: Vec<Vec<u8>>,
                                dao_action: DaoAction| {
            ContexBasedRedeemerCreator::create(move |context: OperationResultContext| {
                PlutusData::ConstrPlutusData(ConstrPlutusData::new(
                    0,
                    vec![
                        dao_action.into_pd(),
                        context.pool_input_idx.into_pd(),
                        POOL_OUT_IDX_IN.into_pd(),
                        PlutusData::new_list(
                            signatures
                                .iter()
                                .map(|signature| {
                                    PlutusData::new_bytes(signature.clone().to_raw_bytes().to_vec())
                                })
                                .collect(),
                        ),
                        PlutusData::new_list(
                            additional_bytes
                                .iter()
                                .map(|additional_bytes_to_add| {
                                    PlutusData::new_bytes(additional_bytes_to_add.clone())
                                })
                                .collect(),
                        ),
                    ],
                ))
            })
        };

        match dao_request.order.dao_action {
            DaoAction::WithdrawTreasury => {
                let treasury_withdraw = TreasuryWithdraw(TreasuryWithdrawOutput {
                    token_x_asset: self.asset_x,
                    token_x_amount: dao_request.order.treasury_x_abs_delta,
                    token_y_asset: self.asset_y,
                    token_y_amount: dao_request.order.treasury_y_abs_delta,
                    // todo: fix in t2t case
                    ada_residue: 0,
                    treasury_script_hash: self.treasury_address,
                });
                Ok((
                    self.clone(),
                    OperationResultBlueprint {
                        outputs: OperationResultOutputs::multiple(requestor_output, vec![treasury_withdraw]),
                        witness_script: Some((
                            dao_validator.clone(),
                            redeemer_creator(
                                self.id,
                                dao_request.order.signatures,
                                dao_request.order.additional_bytes,
                                dao_request.order.dao_action,
                            ),
                        )),
                        order_script_validator: validator,
                        strict_fee: Some(dao_ctx.execution_fee),
                    },
                ))
            }
            _ => Ok((
                self,
                OperationResultBlueprint {
                    outputs: SingleOutput(requestor_output),
                    witness_script: Some((
                        dao_validator.clone(),
                        redeemer_creator(
                            self.id,
                            dao_request.order.signatures,
                            dao_request.order.additional_bytes,
                            dao_request.order.dao_action,
                        ),
                    )),
                    order_script_validator: validator,
                    strict_fee: Some(dao_ctx.execution_fee),
                },
            )),
        }
    }
}

impl AMMOps for RoyaltyPool {
    fn output_amount(
        &self,
        base_asset: TaggedAssetClass<Base>,
        base_amount: TaggedAmount<Base>,
    ) -> TaggedAmount<Quote> {
        classic_cfmm_output_amount(
            self.asset_x,
            self.asset_y,
            self.reserves_x - self.treasury_x - self.first_royalty_x - self.second_royalty_x,
            self.reserves_y - self.treasury_y - self.first_royalty_y - self.second_royalty_y,
            base_asset,
            base_amount,
            self.lp_fee - self.treasury_fee - self.first_royalty_fee - self.second_royalty_fee,
            self.lp_fee - self.treasury_fee - self.first_royalty_fee - self.second_royalty_fee,
        )
    }

    fn reward_lp(
        &self,
        in_x_amount: u64,
        in_y_amount: u64,
    ) -> Option<(TaggedAmount<Lq>, TaggedAmount<Rx>, TaggedAmount<Ry>)> {
        classic_cfmm_reward_lp(
            self.reserves_x - self.treasury_x - self.first_royalty_x - self.second_royalty_x,
            self.reserves_y - self.treasury_y - self.first_royalty_y - self.second_royalty_y,
            self.liquidity,
            in_x_amount,
            in_y_amount,
        )
    }

    fn shares_amount(&self, burned_lq: TaggedAmount<Lq>) -> Option<(TaggedAmount<Rx>, TaggedAmount<Ry>)> {
        classic_cfmm_shares_amount(
            self.reserves_x - self.treasury_x - self.first_royalty_x - self.second_royalty_x,
            self.reserves_y - self.treasury_y - self.first_royalty_y - self.second_royalty_y,
            self.liquidity,
            burned_lq,
        )
    }
}

impl<Ctx> RequiresValidator<Ctx> for RoyaltyPool
where
    Ctx: Has<DeployedValidator<{ RoyaltyPoolV1 as u8 }>>
        + Has<DeployedValidator<{ RoyaltyPoolV1LedgerFixed as u8 }>>
        + Has<DeployedValidator<{ RoyaltyPoolV2 as u8 }>>,
{
    fn get_validator(&self, ctx: &Ctx) -> DeployedValidatorErased {
        match self.ver {
            RoyaltyPoolVer::V1 => ctx
                .select::<DeployedValidator<{ RoyaltyPoolV1 as u8 }>>()
                .erased(),
            RoyaltyPoolVer::V1LedgerFixed => ctx
                .select::<DeployedValidator<{ RoyaltyPoolV1LedgerFixed as u8 }>>()
                .erased(),
            RoyaltyPoolVer::V2 => ctx
                .select::<DeployedValidator<{ RoyaltyPoolV2 as u8 }>>()
                .erased(),
        }
    }
}

impl MakerBehavior for RoyaltyPool {
    fn swap(mut self, input: OnSide<u64>) -> Next<Self, Void> {
        let x = self.asset_x.untag();
        let y = self.asset_y.untag();
        let [base, quote] = order_canonical(x, y);
        let output = match input {
            OnSide::Bid(input) => self
                .output_amount(TaggedAssetClass::new(quote), TaggedAmount::new(input))
                .untag(),
            OnSide::Ask(input) => self
                .output_amount(TaggedAssetClass::new(base), TaggedAmount::new(input))
                .untag(),
        };
        let (
            base_reserves,
            base_treasury,
            base_first_royalty,
            base_second_royalty,
            quote_reserves,
            quote_treasury,
            quote_first_royalty,
            quote_second_royalty,
        ) = if x == base {
            (
                self.reserves_x.as_mut(),
                self.treasury_x.as_mut(),
                self.first_royalty_x.as_mut(),
                self.second_royalty_x.as_mut(),
                self.reserves_y.as_mut(),
                self.treasury_y.as_mut(),
                self.first_royalty_y.as_mut(),
                self.second_royalty_y.as_mut(),
            )
        } else {
            (
                self.reserves_y.as_mut(),
                self.treasury_y.as_mut(),
                self.first_royalty_y.as_mut(),
                self.second_royalty_y.as_mut(),
                self.reserves_x.as_mut(),
                self.treasury_x.as_mut(),
                self.first_royalty_x.as_mut(),
                self.second_royalty_x.as_mut(),
            )
        };
        match input {
            OnSide::Bid(input) => {
                // A user bid means that they wish to buy the base asset for the quote asset, hence
                // pool reserves of base decreases while reserves of quote increase.
                *quote_reserves += input;
                *base_reserves -= output;
                *quote_treasury += (input * self.treasury_fee.numer()) / self.treasury_fee.denom();
                *quote_first_royalty +=
                    (input * self.first_royalty_fee.numer()) / self.first_royalty_fee.denom();
                *quote_second_royalty +=
                    (input * self.second_royalty_fee.numer()) / self.second_royalty_fee.denom();
            }
            OnSide::Ask(input) => {
                // User ask is the opposite; sell the base asset for the quote asset.
                *base_reserves += input;
                *quote_reserves -= output;
                *base_treasury += (input * self.treasury_fee.numer()) / self.treasury_fee.denom();
                *base_first_royalty +=
                    (input * self.first_royalty_fee.numer()) / self.first_royalty_fee.denom();
                *base_second_royalty +=
                    (input * self.second_royalty_fee.numer()) / self.second_royalty_fee.denom();
            }
        }
        Next::Succ(self)
    }
}

impl MarketMaker for RoyaltyPool {
    type U = ExUnits;

    fn static_price(&self) -> SpotPrice {
        let x = self.asset_x.untag();
        let y = self.asset_y.untag();
        let [base, _] = order_canonical(x, y);
        let available_x_reserves =
            (self.reserves_x - self.treasury_x - self.first_royalty_x - self.second_royalty_x).untag();
        let available_y_reserves =
            (self.reserves_y - self.treasury_y - self.first_royalty_y - self.second_royalty_y).untag();
        if available_x_reserves == available_y_reserves {
            AbsolutePrice::new_unsafe(1, 1).into()
        } else {
            if x == base {
                AbsolutePrice::new_unsafe(available_y_reserves, available_x_reserves).into()
            } else {
                AbsolutePrice::new_unsafe(available_x_reserves, available_y_reserves).into()
            }
        }
    }

    fn real_price(&self, input: OnSide<u64>) -> Option<AbsolutePrice> {
        let x = self.asset_x.untag();
        let y = self.asset_y.untag();
        let [base, quote] = order_canonical(x, y);
        let (base, quote) = match input {
            OnSide::Bid(input) => (
                self.output_amount(TaggedAssetClass::new(quote), TaggedAmount::new(input))
                    .untag(),
                input,
            ),
            OnSide::Ask(input) => (
                input,
                self.output_amount(TaggedAssetClass::new(base), TaggedAmount::new(input))
                    .untag(),
            ),
        };
        AbsolutePrice::new(quote, base)
    }

    fn quality(&self) -> PoolQuality {
        PoolQuality::from(self.liquidity.untag())
    }

    fn marginal_cost_hint(&self) -> Self::U {
        self.marginal_cost
    }

    fn is_active(&self) -> bool {
        let lq_bound = (self.reserves_x.untag() * 2) >= self.lq_lower_bound.untag();
        let native_bound = if self.asset_x.is_native() {
            self.reserves_x.untag() >= self.bounds.min_n2t_lovelace
        } else if self.asset_y.is_native() {
            self.reserves_y.untag() >= self.bounds.min_n2t_lovelace
        } else {
            true
        };
        lq_bound && native_bound
    }

    fn liquidity(&self) -> AbsoluteReserves {
        let x = self.asset_x.untag();
        let y = self.asset_y.untag();
        let [base, _] = order_canonical(x, y);
        if base == x {
            AbsoluteReserves {
                base: self.reserves_x.untag(),
                quote: self.reserves_y.untag(),
            }
        } else {
            AbsoluteReserves {
                base: self.reserves_y.untag(),
                quote: self.reserves_x.untag(),
            }
        }
    }

    fn available_liquidity_on_side(&self, worst_price: OnSide<AbsolutePrice>) -> Option<AvailableLiquidity> {
        let sqrt_degree = BigNumber::from(0.5);

        let [base, _] = order_canonical(self.asset_x.untag(), self.asset_y.untag());

        let tradable_x_reserves_int =
            (self.reserves_x - self.treasury_x - self.first_royalty_x - self.second_royalty_x).untag();
        let tradable_x_reserves = BigNumber::from(tradable_x_reserves_int as f64);
        let tradable_y_reserves_int =
            (self.reserves_y - self.treasury_y - self.first_royalty_y - self.second_royalty_y).untag();
        let tradable_y_reserves = BigNumber::from(tradable_y_reserves_int as f64);
        let raw_fee_x = self
            .lp_fee
            .checked_sub(&self.treasury_fee)
            .and_then(|fee| fee.checked_sub(&self.first_royalty_fee))
            .and_then(|fee| fee.checked_sub(&self.second_royalty_fee))?;
        let fee_x = BigNumber::from(raw_fee_x.to_f64()?);
        let raw_fee_y = self
            .lp_fee
            .checked_sub(&self.treasury_fee)
            .and_then(|fee| fee.checked_sub(&self.first_royalty_fee))
            .and_then(|fee| fee.checked_sub(&self.second_royalty_fee))?;
        let fee_y = BigNumber::from(raw_fee_y.to_f64()?);
        let bid_price = BigNumber::from(*worst_price.unwrap().denom() as f64)
            / BigNumber::from(*worst_price.unwrap().numer() as f64);
        let ask_price = BigNumber::from(*worst_price.unwrap().numer() as f64)
            / BigNumber::from(*worst_price.unwrap().denom() as f64);

        let (
            tradable_reserves_quote_int,
            tradable_reserves_base,
            tradable_reserves_quote,
            total_fee_mult,
            avg_price,
        ) = match worst_price {
            OnSide::Bid(_) if base == self.asset_x.untag() => (
                tradable_x_reserves_int,
                tradable_y_reserves,
                tradable_x_reserves,
                fee_y,
                bid_price,
            ),
            OnSide::Bid(_) => (
                tradable_y_reserves_int,
                tradable_x_reserves,
                tradable_y_reserves,
                fee_x,
                bid_price,
            ),
            OnSide::Ask(_) if base == self.asset_x.untag() => (
                tradable_y_reserves_int,
                tradable_x_reserves,
                tradable_y_reserves,
                fee_x,
                ask_price,
            ),
            OnSide::Ask(_) => (
                tradable_x_reserves_int,
                tradable_y_reserves,
                tradable_x_reserves,
                fee_y,
                ask_price,
            ),
        };

        let lq_balance = (tradable_reserves_base.clone() * tradable_reserves_quote.clone()).pow(&sqrt_degree);

        let p1 = (avg_price.div(total_fee_mult.clone()) * lq_balance.clone()
            / tradable_reserves_quote.clone())
        .pow(&(BigNumber::from(2)));
        let p1_sqrt = p1.clone().pow(&sqrt_degree);
        let x1 = lq_balance.clone() / p1_sqrt.clone();
        let y1 = lq_balance.clone() * p1_sqrt.clone();

        let input_amount = (x1.clone() - tradable_reserves_base.clone()) / total_fee_mult;
        let output_amount = tradable_reserves_quote - y1.clone();

        let input_amount_val = <u64>::try_from(input_amount.value.to_int().value()).ok()?;
        let output_amount_val = <u64>::try_from(output_amount.value.to_int().value()).ok()?;
        let remain_quote = tradable_reserves_quote_int.checked_sub(output_amount_val)?;

        let capped_output_amount = match UNTOUCHABLE_LOVELACE_AMOUNT.checked_sub(remain_quote) {
            Some(_) => tradable_reserves_quote_int.checked_sub(UNTOUCHABLE_LOVELACE_AMOUNT)?,
            None => output_amount_val,
        };

        Some(AvailableLiquidity {
            input: input_amount_val,
            output: capped_output_amount,
        })
    }

    fn estimated_trade(&self, input: OnSide<u64>) -> Option<AvailableLiquidity> {
        let x = self.asset_x.untag();
        let y = self.asset_y.untag();
        let [base, quote] = order_canonical(x, y);
        let output = match input {
            OnSide::Bid(input) => self
                .output_amount(TaggedAssetClass::new(quote), TaggedAmount::new(input))
                .untag(),
            OnSide::Ask(input) => self
                .output_amount(TaggedAssetClass::new(base), TaggedAmount::new(input))
                .untag(),
        };
        Some(AvailableLiquidity {
            input: input.unwrap(),
            output,
        })
    }
}

impl Stable for RoyaltyPool {
    type StableId = PoolId;

    fn stable_id(&self) -> Self::StableId {
        self.id
    }

    fn is_quasi_permanent(&self) -> bool {
        true
    }
}

impl<Ctx> ApplyOrder<OnChainRoyaltyWithdraw, Ctx> for RoyaltyPool
where
    Ctx: Has<DeployedValidator<{ RoyaltyPoolRoyaltyWithdraw as u8 }>>
        + Has<DeployedValidator<{ RoyaltyPoolRoyaltyWithdrawV2 as u8 }>>
        + Has<DeployedValidator<{ RoyaltyPoolRoyaltyWithdrawLedgerFixed as u8 }>>
        + Has<DeployedValidator<{ RoyaltyPoolV1RoyaltyWithdrawRequest as u8 }>>
        + Has<DeployedValidator<{ RoyaltyPoolV2RoyaltyWithdrawRequest as u8 }>>
        + Has<RoyaltyWithdrawContext>,
{
    type Result = RoyaltyWithdrawOutput;

    fn apply_order(
        mut self,
        royalty_withdraw: OnChainRoyaltyWithdraw,
        ctx: Ctx,
    ) -> Result<
        (Self, OperationResultBlueprint<RoyaltyWithdrawOutput>),
        ApplyOrderError<OnChainRoyaltyWithdraw>,
    > {
        let order = royalty_withdraw.order.clone();
        let order_validator = royalty_withdraw.get_validator(&ctx);
        let pool_order_validator = if self.ver == RoyaltyPoolVer::V1 {
            ctx.select::<DeployedValidator<{ RoyaltyPoolRoyaltyWithdraw as u8 }>>()
                .erased()
        } else if self.ver == RoyaltyPoolVer::V1LedgerFixed {
            ctx.select::<DeployedValidator<{ RoyaltyPoolRoyaltyWithdrawLedgerFixed as u8 }>>()
                .erased()
        } else {
            ctx.select::<DeployedValidator<{ RoyaltyPoolRoyaltyWithdrawV2 as u8 }>>()
                .erased()
        };
        let royalty_withdraw_context: RoyaltyWithdrawContext = ctx.get();

        let data_to_sign = DataToSign {
            withdraw_data: royalty_withdraw.clone().order.into(),
            pool_nonce: self.nonce,
        };

        let mut message_to_sign = order.additional_bytes.clone();
        message_to_sign.extend(&data_to_sign.clone().into_pd().to_cbor_bytes());
        let hashed_data_to_sign = blake2b224(&data_to_sign.into_pd().to_cbor_bytes());
        let mut hashed_message_to_sign = order.additional_bytes.clone();
        hashed_message_to_sign.extend(&hashed_data_to_sign);

        if self.ver == RoyaltyPoolVer::V1 || self.ver == RoyaltyPoolVer::V1LedgerFixed {
            let public_key: PublicKey = self.first_royalty_pub_key.try_into().unwrap();

            let hashed = if public_key.verify(&message_to_sign, &order.signature) {
                false
            } else if public_key.verify(&hashed_message_to_sign, &order.signature) {
                true
            } else {
                return Err(ApplyOrderError::verification_failed(
                    royalty_withdraw,
                    format!("signature_is_incorrect"),
                ));
            };

            self.reserves_x = self
                .reserves_x
                .checked_sub(&order.withdraw_royalty_x)
                .ok_or(ApplyOrderError::incompatible(royalty_withdraw.clone()))?;

            self.first_royalty_x = self
                .first_royalty_x
                .checked_sub(&order.withdraw_royalty_x)
                .ok_or(ApplyOrderError::incompatible(royalty_withdraw.clone()))?;

            self.reserves_y = self
                .reserves_y
                .checked_sub(&order.withdraw_royalty_y)
                .ok_or(ApplyOrderError::incompatible(royalty_withdraw.clone()))?;

            self.first_royalty_y = self
                .first_royalty_y
                .checked_sub(&order.withdraw_royalty_y)
                .ok_or(ApplyOrderError::incompatible(royalty_withdraw.clone()))?;

            self.nonce += 1;

            let royalty_output = RoyaltyWithdrawOutput {
                token_x_asset: self.asset_x,
                token_x_amount: order.withdraw_royalty_x,
                token_y_asset: self.asset_y,
                token_y_amount: order.withdraw_royalty_y,
                ada_residue: order.init_ada_value - order.fee,
                redeemer_pkh: order.royalty_pub_key_hash,
            };

            let redeemer = if self.ver == RoyaltyPoolVer::V1 {
                ContexBasedRedeemerCreator::create(move |context: OperationResultContext| {
                    ConstrPlutusData::new(
                        0,
                        vec![
                            PlutusData::Integer(BigInteger::from(context.pool_input_idx)),
                            PlutusData::Integer(BigInteger::from(context.order_input_idx)),
                        ],
                    )
                    .into_pd()
                })
            } else {
                ContexBasedRedeemerCreator::create(move |context: OperationResultContext| {
                    ConstrPlutusData::new(
                        0,
                        vec![
                            PlutusData::Integer(BigInteger::from(context.pool_input_idx)),
                            PlutusData::Integer(BigInteger::from(context.order_input_idx)),
                            if hashed { AIKEN_TRUE } else { AIKEN_FALSE },
                        ],
                    )
                    .into_pd()
                })
            };

            let blueprint = OperationResultBlueprint {
                outputs: SingleOutput(royalty_output),
                witness_script: Some((pool_order_validator.clone(), redeemer)),
                order_script_validator: order_validator,
                strict_fee: Some(royalty_withdraw_context.transaction_fee),
            };

            Ok((self, blueprint))
        } else {
            let (public_key_idx, hashed) = {
                let first_public_key: PublicKey = self.first_royalty_pub_key.try_into().unwrap();
                let second_public_key: PublicKey = self.second_royalty_pub_key.try_into().unwrap();

                if first_public_key.verify(&message_to_sign, &order.signature) {
                    (0, false)
                } else if first_public_key.verify(&hashed_message_to_sign, &order.signature) {
                    (0, true)
                } else if second_public_key.verify(&message_to_sign, &order.signature) {
                    (1, false)
                } else if second_public_key.verify(&hashed_message_to_sign, &order.signature) {
                    (1, true)
                } else {
                    return Err(ApplyOrderError::verification_failed(
                        royalty_withdraw,
                        format!("signature is incorrect for both keys"),
                    ));
                }
            };

            if public_key_idx == 0 {
                self.first_royalty_x = self
                    .first_royalty_x
                    .checked_sub(&order.withdraw_royalty_x)
                    .ok_or(ApplyOrderError::incompatible(royalty_withdraw.clone()))?;

                self.first_royalty_y = self
                    .first_royalty_y
                    .checked_sub(&order.withdraw_royalty_y)
                    .ok_or(ApplyOrderError::incompatible(royalty_withdraw.clone()))?;
            } else {
                self.second_royalty_x = self
                    .second_royalty_x
                    .checked_sub(&order.withdraw_royalty_x)
                    .ok_or(ApplyOrderError::incompatible(royalty_withdraw.clone()))?;

                self.second_royalty_y = self
                    .second_royalty_y
                    .checked_sub(&order.withdraw_royalty_y)
                    .ok_or(ApplyOrderError::incompatible(royalty_withdraw.clone()))?;
            }

            self.reserves_x = self
                .reserves_x
                .checked_sub(&order.withdraw_royalty_x)
                .ok_or(ApplyOrderError::incompatible(royalty_withdraw.clone()))?;

            self.reserves_y = self
                .reserves_y
                .checked_sub(&order.withdraw_royalty_y)
                .ok_or(ApplyOrderError::incompatible(royalty_withdraw.clone()))?;

            self.nonce += 1;

            let royalty_output = RoyaltyWithdrawOutput {
                token_x_asset: self.asset_x,
                token_x_amount: order.withdraw_royalty_x,
                token_y_asset: self.asset_y,
                token_y_amount: order.withdraw_royalty_y,
                ada_residue: order.init_ada_value - order.fee,
                redeemer_pkh: order.royalty_pub_key_hash,
            };

            let blueprint = OperationResultBlueprint {
                outputs: SingleOutput(royalty_output),
                witness_script: Some((
                    pool_order_validator.clone(),
                    ContexBasedRedeemerCreator::create(move |context: OperationResultContext| {
                        ConstrPlutusData::new(
                            0,
                            vec![
                                PlutusData::Integer(BigInteger::from(context.pool_input_idx)),
                                PlutusData::Integer(BigInteger::from(context.order_input_idx)),
                                PlutusData::Integer(BigInteger::from(public_key_idx)),
                                if hashed { AIKEN_TRUE } else { AIKEN_FALSE },
                            ],
                        )
                        .into_pd()
                    }),
                )),
                order_script_validator: order_validator,
                strict_fee: Some(royalty_withdraw_context.transaction_fee),
            };

            Ok((self, blueprint))
        }
    }
}

impl IntoLedger<TransactionOutput, ImmutablePoolUtxo> for RoyaltyPool {
    fn into_ledger(self, mut immut_pool: ImmutablePoolUtxo) -> TransactionOutput {
        let mut ma = MultiAsset::new();
        let coins = if self.asset_x.is_native() {
            let Token(policy, name) = self.asset_y.untag().into_token().unwrap();
            ma.set(policy, name.into(), self.reserves_y.untag());
            self.reserves_x.untag()
        } else if self.asset_y.is_native() {
            let Token(policy, name) = self.asset_x.untag().into_token().unwrap();
            ma.set(policy, name.into(), self.reserves_x.untag());
            self.reserves_y.untag()
        } else {
            let Token(policy_x, name_x) = self.asset_x.untag().into_token().unwrap();
            ma.set(policy_x, name_x.into(), self.reserves_x.untag());
            let Token(policy_y, name_y) = self.asset_y.untag().into_token().unwrap();
            ma.set(policy_y, name_y.into(), self.reserves_y.untag());
            immut_pool.value
        };
        let Token(policy_lq, name_lq) = self.asset_lq.untag().into_token().unwrap();
        let Token(nft_lq, name_nft) = self.id.into();
        ma.set(policy_lq, name_lq.into(), MAX_LQ_CAP - self.liquidity.untag());
        ma.set(nft_lq, name_nft.into(), 1);

        if self.ver == RoyaltyPoolVer::V1 || self.ver == RoyaltyPoolVer::V1LedgerFixed {
            if let Some(DatumOption::Datum { datum, .. }) = &mut immut_pool.datum_option {
                unsafe_update_pd_royalty(
                    datum,
                    // royalty pool have the same lp_fee_x and lp_fee_y values
                    *self.lp_fee.numer(),
                    *self.treasury_fee.numer(),
                    *self.first_royalty_fee.numer(),
                    self.treasury_x.untag(),
                    self.treasury_y.untag(),
                    self.first_royalty_x.untag(),
                    self.first_royalty_y.untag(),
                    self.treasury_address,
                    self.admin_address,
                    self.nonce,
                );
            }
        } else if self.ver == RoyaltyPoolVer::V2 {
            if let Some(DatumOption::Datum { datum, .. }) = &mut immut_pool.datum_option {
                unsafe_update_pd_royalty_v2(
                    datum,
                    // royalty pool have the same lp_fee_x and lp_fee_y values
                    *self.lp_fee.numer(),
                    *self.treasury_fee.numer(),
                    *self.first_royalty_fee.numer(),
                    *self.second_royalty_fee.numer(),
                    self.treasury_x.untag(),
                    self.treasury_y.untag(),
                    self.first_royalty_x.untag(),
                    self.first_royalty_y.untag(),
                    self.second_royalty_x.untag(),
                    self.second_royalty_y.untag(),
                    self.treasury_address,
                    self.admin_address,
                    self.nonce,
                );
            }
        }

        if let Some(stake_part_script_hash) = self.stake_part_script_hash {
            immut_pool
                .address
                .update_staking_cred(Credential::new_script(stake_part_script_hash))
        }

        TransactionOutput::new_conway_format_tx_out(ConwayFormatTxOut {
            address: immut_pool.address,
            amount: Value::new(coins, ma),
            datum_option: immut_pool.datum_option,
            script_reference: immut_pool.script_reference,
            encodings: None,
        })
    }
}

mod tests {
    use crate::creds::OperatorCred;
    use crate::data::pool::PoolValidation;
    use crate::data::royalty_withdraw_request::{OnChainRoyaltyWithdraw, RoyaltyWithdrawOrderValidation};
    use crate::deployment::ProtocolValidator::{
        ConstFnPoolFeeSwitch, ConstFnPoolFeeSwitchBiDirFee, ConstFnPoolFeeSwitchV2, ConstFnPoolV1,
        ConstFnPoolV2, RoyaltyPoolV1, RoyaltyPoolV1LedgerFixed, RoyaltyPoolV1RoyaltyWithdrawRequest,
        RoyaltyPoolV2, RoyaltyPoolV2RoyaltyWithdrawRequest,
    };
    use crate::deployment::{DeployedScriptInfo, DeployedValidators, ProtocolScriptHashes};
    use crate::handler_context::{ConsumedIdentifiers, ConsumedInputs, ProducedIdentifiers};
    use cml_chain::transaction::TransactionOutput;
    use cml_core::serialization::Deserialize;
    use cml_crypto::{Ed25519KeyHash, TransactionHash};
    use spectrum_cardano_lib::{OutputRef, Token};
    use spectrum_offchain::data::small_vec::SmallVec;
    use spectrum_offchain::domain::Has;
    use spectrum_offchain::ledger::TryFromLedger;
    use type_equalities::IsEqual;

    struct Context {
        oref: OutputRef,
        royalty_pool: DeployedScriptInfo<{ RoyaltyPoolV1 as u8 }>,
        royalty_pool_ledger_fixed: DeployedScriptInfo<{ RoyaltyPoolV1LedgerFixed as u8 }>,
        royalty_pool_v2: DeployedScriptInfo<{ RoyaltyPoolV2 as u8 }>,
        const_fn_pool_v1: DeployedScriptInfo<{ ConstFnPoolV1 as u8 }>,
        const_fn_pool_v2: DeployedScriptInfo<{ ConstFnPoolV2 as u8 }>,
        fee_switch_v1: DeployedScriptInfo<{ ConstFnPoolFeeSwitch as u8 }>,
        fee_switch_v2: DeployedScriptInfo<{ ConstFnPoolFeeSwitchV2 as u8 }>,
        fee_switch_bi_dir: DeployedScriptInfo<{ ConstFnPoolFeeSwitchBiDirFee as u8 }>,
        royalty_withdraw: DeployedScriptInfo<{ RoyaltyPoolV1RoyaltyWithdrawRequest as u8 }>,
        royalty_withdraw_v2: DeployedScriptInfo<{ RoyaltyPoolV2RoyaltyWithdrawRequest as u8 }>,
        cred: OperatorCred,
        consumed_inputs: ConsumedInputs,
        consumed_identifiers: ConsumedIdentifiers<Token>,
        produced_identifiers: ProducedIdentifiers<Token>,
        pool_validation: PoolValidation,
        royalty_validation: RoyaltyWithdrawOrderValidation,
    }

    impl Has<DeployedScriptInfo<{ RoyaltyPoolV1 as u8 }>> for Context {
        fn select<U: IsEqual<DeployedScriptInfo<{ RoyaltyPoolV1 as u8 }>>>(
            &self,
        ) -> DeployedScriptInfo<{ RoyaltyPoolV1 as u8 }> {
            self.royalty_pool
        }
    }

    impl Has<DeployedScriptInfo<{ RoyaltyPoolV1LedgerFixed as u8 }>> for Context {
        fn select<U: IsEqual<DeployedScriptInfo<{ RoyaltyPoolV1LedgerFixed as u8 }>>>(
            &self,
        ) -> DeployedScriptInfo<{ RoyaltyPoolV1LedgerFixed as u8 }> {
            self.royalty_pool_ledger_fixed
        }
    }

    impl Has<DeployedScriptInfo<{ RoyaltyPoolV2 as u8 }>> for Context {
        fn select<U: IsEqual<DeployedScriptInfo<{ RoyaltyPoolV2 as u8 }>>>(
            &self,
        ) -> DeployedScriptInfo<{ RoyaltyPoolV2 as u8 }> {
            self.royalty_pool_v2
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

    impl Has<DeployedScriptInfo<{ RoyaltyPoolV2RoyaltyWithdrawRequest as u8 }>> for Context {
        fn select<U: IsEqual<DeployedScriptInfo<{ RoyaltyPoolV2RoyaltyWithdrawRequest as u8 }>>>(
            &self,
        ) -> DeployedScriptInfo<{ RoyaltyPoolV2RoyaltyWithdrawRequest as u8 }> {
            self.royalty_withdraw_v2
        }
    }

    impl Has<OutputRef> for Context {
        fn select<U: IsEqual<OutputRef>>(&self) -> OutputRef {
            self.oref
        }
    }

    impl Has<RoyaltyWithdrawOrderValidation> for Context {
        fn select<U: IsEqual<RoyaltyWithdrawOrderValidation>>(&self) -> RoyaltyWithdrawOrderValidation {
            self.royalty_validation
        }
    }

    impl Has<PoolValidation> for Context {
        fn select<U: IsEqual<PoolValidation>>(&self) -> PoolValidation {
            self.pool_validation
        }
    }

    const POOL_UTXO: &str = "a300583930156cf166f3cfea6b6fcabd07a3a0f8217aef135b2859eb01deba6948b2f6abf60ccde92eae1a2f4fdf65f2eaf6208d872c6f0e597cc10b0701821a004c4b40a4581c6e917b8b965078a39804a6313e5be73535612421acd70aa83f0ec200a158201fa39f0acfe45739a9c7abef0e51a27ab8bdb20e62b2da3521aacb24388698601b7fffffff75db360a581c74591babd5070801f8a45658d356fc8e957aea9dd75612ee6e6884e7a145746f6b656e1a0ffdd24b581cd8eb52caf3289a2880288b23141ce3d2a7025dcf76f26fd5659add06a15820cf6eb4eb6f2ca2a35298388b71e524716481196d0abb8533ddfdd29b84f834e201581cf357c6f00f0496fcd01851a7a8d909a1d9d1c9d7ba9bc021ac3bc3fea14d636e74546f6b656e746f6b656e1b00000004a95d6d1d028201d818590180d8799fd8799f581cd8eb52caf3289a2880288b23141ce3d2a7025dcf76f26fd5659add065820cf6eb4eb6f2ca2a35298388b71e524716481196d0abb8533ddfdd29b84f834e2ffd8799f581cf357c6f00f0496fcd01851a7a8d909a1d9d1c9d7ba9bc021ac3bc3fe4d636e74546f6b656e746f6b656effd8799f581c74591babd5070801f8a45658d356fc8e957aea9dd75612ee6e6884e745746f6b656effd8799f581c6e917b8b965078a39804a6313e5be73535612421acd70aa83f0ec20058201fa39f0acfe45739a9c7abef0e51a27ab8bdb20e62b2da3521aacb2438869860ff1a0001831c1832183218320000000000009fd8799fd87a9f581ce7d070220666eee395115a938ac86fb69773fa8b8183ff0aad1a47dbffffff581c75c4570eb625ae881b32a34c52b159f6f3f3f2c7aaabf5bac4688133582072c68f905716a5f59a0ee2552ab68559f42287d335396d8f430da98e96c5009c5820c2d5d602ef90b3bc522a8bd6723b3fd94fbcfcc103997fe5ac769e369ab9ea1800ff";

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
        let royalty_withdraw_validation = RoyaltyWithdrawOrderValidation {
            min_ada_in_royalty_output: 1_000_000,
        };
        let ctx = Context {
            oref,
            royalty_pool: scripts.royalty_pool_v1,
            royalty_pool_ledger_fixed: scripts.royalty_pool_v1_ledger_fixed,
            royalty_pool_v2: scripts.royalty_pool_v2,
            const_fn_pool_v1: scripts.const_fn_pool_v1,
            const_fn_pool_v2: scripts.const_fn_pool_v2,
            fee_switch_v1: scripts.const_fn_pool_fee_switch,
            fee_switch_v2: scripts.const_fn_pool_fee_switch_v2,
            fee_switch_bi_dir: scripts.const_fn_pool_fee_switch_bidir_fee,
            royalty_withdraw: scripts.royalty_pool_withdraw_request,
            royalty_withdraw_v2: scripts.royalty_pool_v2_withdraw_request,
            cred: OperatorCred(Ed25519KeyHash::from([0u8; 28])),
            consumed_inputs: SmallVec::new(vec![oref].into_iter()).into(),
            consumed_identifiers: Default::default(),
            produced_identifiers: Default::default(),
            pool_validation: PoolValidation {
                min_n2t_lovelace: 10,
                min_t2t_lovelace: 10,
            },
            royalty_validation: royalty_withdraw_validation,
        };
        let bearer = TransactionOutput::from_cbor_bytes(&*hex::decode(POOL_UTXO).unwrap()).unwrap();
        let pool = OnChainRoyaltyWithdraw::try_from_ledger(&bearer, &ctx);
        assert_eq!(pool.is_some(), true)
    }
}
