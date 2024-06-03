use cml_chain::plutus::PlutusData;

use spectrum_cardano_lib::plutus_data::{ConstrPlutusDataExtension, PlutusDataExtension};
use spectrum_cardano_lib::types::TryFromPData;
use spectrum_cardano_lib::{TaggedAmount, TaggedAssetClass};

use crate::data::order::PoolNft;
use crate::data::pool::{Lq, Rx, Ry};

pub struct FeeSwitchBidirectionalPoolConfig {
    pub pool_nft: TaggedAssetClass<PoolNft>,
    pub asset_x: TaggedAssetClass<Rx>,
    pub asset_y: TaggedAssetClass<Ry>,
    pub asset_lq: TaggedAssetClass<Lq>,
    pub lp_fee_num_x: u64,
    pub lp_fee_num_y: u64,
    pub treasury_fee_num: u64,
    pub treasury_x: u64,
    pub treasury_y: u64,
    pub lq_lower_bound: TaggedAmount<Rx>,
}

impl TryFromPData for FeeSwitchBidirectionalPoolConfig {
    fn try_from_pd(data: PlutusData) -> Option<Self> {
        let mut cpd = data.into_constr_pd()?;
        Some(Self {
            pool_nft: TaggedAssetClass::try_from_pd(cpd.take_field(0)?)?,
            asset_x: TaggedAssetClass::try_from_pd(cpd.take_field(1)?)?,
            asset_y: TaggedAssetClass::try_from_pd(cpd.take_field(2)?)?,
            asset_lq: TaggedAssetClass::try_from_pd(cpd.take_field(3)?)?,
            lp_fee_num_x: cpd.take_field(4)?.into_u64()?,
            lp_fee_num_y: cpd.take_field(5)?.into_u64()?,
            treasury_fee_num: cpd.take_field(6)?.into_u64()?,
            treasury_x: cpd.take_field(7)?.into_u64()?,
            treasury_y: cpd.take_field(8)?.into_u64()?,
            lq_lower_bound: TaggedAmount::new(cpd.take_field(10).and_then(|pd| pd.into_u64()).unwrap_or(0)),
        })
    }
}
