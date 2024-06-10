use std::fmt::{Display, Formatter};

use cml_chain::address::Address;
use cml_chain::transaction::TransactionInput;
use cml_chain::PolicyId;
use cml_crypto::{RawBytesEncoding, TransactionHash};
use num_rational::Ratio;
use rand::{thread_rng, RngCore};

use spectrum_cardano_lib::{AssetClass, AssetName, OutputRef, TaggedAssetClass, Token};

use crate::data::order::PoolNft;

pub mod deposit;
pub mod limit_swap;
pub mod operation_output;
pub mod order;
pub mod pool;
pub mod redeem;

pub mod ref_scripts;

pub mod balance_order;
pub mod balance_pool;
pub mod cfmm_pool;
pub mod fee_switch_bidirectional_fee;
pub mod fee_switch_pool;
pub mod pair;
mod stable_pool_t2t;
mod stable_order;

#[repr(transparent)]
#[derive(
    Debug,
    Copy,
    Clone,
    Eq,
    PartialEq,
    Ord,
    PartialOrd,
    Hash,
    derive_more::From,
    derive_more::Into,
    derive_more::Display,
)]
pub struct OnChainOrderId(OutputRef);

impl From<TransactionInput> for OnChainOrderId {
    fn from(value: TransactionInput) -> Self {
        Self(OutputRef::from(value))
    }
}

impl OnChainOrderId {
    pub fn new(tx: TransactionHash, index: u64) -> Self {
        Self((tx, index).into())
    }
}

#[repr(transparent)]
#[derive(Debug, Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Hash, derive_more::From, derive_more::Into)]
pub struct PoolId(Token);

impl PoolId {
    pub fn random() -> PoolId {
        let mut bf = [0u8; 28];
        thread_rng().fill_bytes(&mut bf);
        let mp = PolicyId::from(bf);
        let tn = AssetName::utf8_unsafe(String::from("nft"));
        PoolId((mp, tn))
    }
}

impl Display for PoolId {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(format!("{}.{}", self.0 .0, self.0 .1).as_str())
    }
}

impl Into<[u8; 60]> for PoolId {
    fn into(self) -> [u8; 60] {
        let mut bf = [0u8; 60];
        let (policy, an) = self.0;
        policy.to_raw_bytes().into_iter().enumerate().for_each(|(ix, i)| {
            bf[ix] = *i;
        });
        an.padded_bytes().into_iter().enumerate().for_each(|(ix, i)| {
            bf[ix + PolicyId::BYTE_COUNT] = i;
        });
        bf
    }
}

impl TryFrom<TaggedAssetClass<PoolNft>> for PoolId {
    type Error = ();
    fn try_from(value: TaggedAssetClass<PoolNft>) -> Result<Self, Self::Error> {
        Ok(PoolId(AssetClass::from(value).into_token().ok_or(())?))
    }
}

#[repr(transparent)]
#[derive(Debug, Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Hash, derive_more::From, derive_more::Into)]
pub struct PoolStateVer(OutputRef);

impl Display for PoolStateVer {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct ExecutorFeePerToken(Ratio<u128>, pub AssetClass);

impl ExecutorFeePerToken {
    pub fn new(rational: Ratio<u128>, ac: AssetClass) -> Self {
        Self(rational, ac)
    }
    pub fn value(&self) -> Ratio<u128> {
        self.0
    }
}
