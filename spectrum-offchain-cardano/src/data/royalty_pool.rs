use crate::data::order::PoolNft;
use crate::data::pool::{Lq, Rx, Ry};
use cml_chain::plutus::PlutusData;
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
};

impl TryFromPData for RoyaltyPoolConfig {
    fn try_from_pd(data: PlutusData) -> Option<Self> {
        let mut cpd = data.into_constr_pd()?;
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
        })
    }
}
