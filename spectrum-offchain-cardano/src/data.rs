use cml_crypto::TransactionHash;
use num_rational::Ratio;

use spectrum_cardano_lib::{AssetClass, OutputRef, TaggedAssetClass, Token};

use crate::data::order::PoolNft;

pub mod limit_swap;
pub mod order;

#[derive(Debug, Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Hash, derive_more::From, derive_more::Into)]
pub struct OnChainOrderId(OutputRef);

impl OnChainOrderId {
    pub fn new(tx: TransactionHash, index: u64) -> Self {
        Self((tx, index))
    }
}

#[derive(Debug, Clone, Eq, PartialEq, Ord, PartialOrd, Hash, derive_more::From, derive_more::Into)]
pub struct PoolId(Token);

impl TryFrom<TaggedAssetClass<PoolNft>> for PoolId {
    type Error = ();
    fn try_from(value: TaggedAssetClass<PoolNft>) -> Result<Self, Self::Error> {
        Ok(PoolId(value.into().into_token().ok_or(())?))
    }
}

#[derive(Debug, Clone)]
pub struct ExecutorFeePerToken(Ratio<u64>, AssetClass);

impl ExecutorFeePerToken {
    pub fn new(rational: Ratio<u64>, ac: AssetClass) -> Self {
        Self(rational, ac)
    }
}
