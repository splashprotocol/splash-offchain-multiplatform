use std::marker::PhantomData;

use cml_chain::address::Address;
use cml_chain::plutus::PlutusData;
use cml_chain::transaction::TransactionOutput;
use num_rational::Ratio;

use spectrum_cardano_lib::plutus_data::{ConstrPlutusDataExtension, DatumExtension, PlutusDataExtension};
use spectrum_cardano_lib::transaction::TransactionOutputExtension;
use spectrum_cardano_lib::types::TryFromPData;
use spectrum_cardano_lib::value::ValueExtension;
use spectrum_cardano_lib::{OutputRef, TaggedAmount, TaggedAssetClass};
use spectrum_offchain::data::OnChainEntity;
use spectrum_offchain::ledger::TryFromLedger;

use crate::constants::{CFMM_LP_FEE_DEN, POOL_ADDR_V1};
use crate::data::order::PoolNft;
use crate::data::{PoolId, PoolStateVer};

pub struct Rx;
pub struct Ry;
pub struct Lq;

pub trait ProtocolVer {
    fn address() -> Address;
}

pub struct V1;

impl ProtocolVer for V1 {
    fn address() -> Address {
        POOL_ADDR_V1.clone()
    }
}

pub struct CFMMPool<Ver> {
    pub id: PoolId,
    pub state_ver: PoolStateVer,
    pub reserves_x: TaggedAmount<Rx>,
    pub reserves_y: TaggedAmount<Ry>,
    pub liquidity: TaggedAmount<Lq>,
    pub asset_x: TaggedAssetClass<Rx>,
    pub asset_y: TaggedAssetClass<Ry>,
    pub asset_lq: TaggedAssetClass<Lq>,
    pub lp_fee: Ratio<u64>,
    pub lq_lower_bound: TaggedAmount<Lq>,
    version_pd: PhantomData<Ver>,
}

impl<Ver> OnChainEntity for CFMMPool<Ver> {
    type TEntityId = PoolId;
    type TStateId = PoolStateVer;
    fn get_self_ref(&self) -> Self::TEntityId {
        self.id
    }
    fn get_self_state_ref(&self) -> Self::TStateId {
        self.state_ver
    }
}

impl TryFromLedger<TransactionOutput, OutputRef> for CFMMPool<V1> {
    fn try_from_ledger(repr: TransactionOutput, ctx: OutputRef) -> Option<Self> {
        let value = repr.amount().clone();
        let conf = CFMMPoolConfig::try_from_pd(repr.into_datum()?.into_pd()?)?;
        Some(Self {
            id: PoolId::try_from(conf.pool_nft).ok()?,
            state_ver: PoolStateVer::from(ctx),
            reserves_x: TaggedAmount::unsafe_tag(value.amount_of(conf.asset_x.into())?),
            reserves_y: TaggedAmount::unsafe_tag(value.amount_of(conf.asset_y.into())?),
            liquidity: TaggedAmount::unsafe_tag(value.amount_of(conf.asset_lq.into())?),
            asset_x: conf.asset_x,
            asset_y: conf.asset_y,
            asset_lq: conf.asset_lq,
            lp_fee: Ratio::new(conf.lp_fee_num, CFMM_LP_FEE_DEN),
            lq_lower_bound: conf.lq_lower_bound,
            version_pd: PhantomData::default(),
        })
    }
}

pub struct CFMMPoolConfig {
    pool_nft: TaggedAssetClass<PoolNft>,
    asset_x: TaggedAssetClass<Rx>,
    asset_y: TaggedAssetClass<Ry>,
    asset_lq: TaggedAssetClass<Lq>,
    lp_fee_num: u64,
    lq_lower_bound: TaggedAmount<Lq>,
}

impl TryFromPData for CFMMPoolConfig {
    fn try_from_pd(data: PlutusData) -> Option<Self> {
        let mut cpd = data.into_constr_pd()?;
        Some(Self {
            pool_nft: TaggedAssetClass::try_from_pd(cpd.take_field(0)?)?,
            asset_x: TaggedAssetClass::try_from_pd(cpd.take_field(1)?)?,
            asset_y: TaggedAssetClass::try_from_pd(cpd.take_field(2)?)?,
            asset_lq: TaggedAssetClass::try_from_pd(cpd.take_field(3)?)?,
            lp_fee_num: cpd.take_field(4)?.into_u64()?,
            lq_lower_bound: TaggedAmount::unsafe_tag(
                cpd.take_field(6).and_then(|pd| pd.into_u64()).unwrap_or(0),
            ),
        })
    }
}
