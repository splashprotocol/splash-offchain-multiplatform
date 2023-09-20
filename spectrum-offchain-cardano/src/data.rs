use std::fmt::{Display, Formatter};

use cml_chain::transaction::{TransactionInput, TransactionOutput};
use cml_crypto::TransactionHash;
use num_rational::Ratio;

use spectrum_cardano_lib::{AssetClass, OutputRef, TaggedAssetClass, Token};
use spectrum_offchain::data::{OnChainEntity, SpecializedOrder};

use crate::data::order::PoolNft;

pub mod batcher_output;
pub mod limit_swap;
pub mod operation_output;
pub mod order;
pub mod pool;

/// For persistent on-chain entities (e.g. pools) we want to carry initial utxo.
#[derive(Debug, Clone)]
pub struct OnChain<T> {
    pub value: T,
    pub source: TransactionOutput,
}

impl<T> OnChain<T> {
    pub fn map<F, A>(self, f: F) -> OnChain<A>
    where
        F: FnOnce(T) -> A,
    {
        let OnChain { value, source } = self;
        OnChain {
            value: f(value),
            source,
        }
    }
}

impl<T> Display for OnChain<T>
where
    T: Display,
{
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(format!("OnChain({})", self.value).as_str())
    }
}

impl<T> OnChainEntity for OnChain<T>
where
    T: OnChainEntity,
{
    type TEntityId = T::TEntityId;
    type TStateId = T::TStateId;

    fn get_self_ref(&self) -> Self::TEntityId {
        self.value.get_self_ref()
    }
    fn get_self_state_ref(&self) -> Self::TStateId {
        self.value.get_self_state_ref()
    }
}

impl<T> SpecializedOrder for OnChain<T>
where
    T: SpecializedOrder,
{
    type TOrderId = T::TOrderId;
    type TPoolId = T::TPoolId;

    fn get_self_ref(&self) -> Self::TOrderId {
        self.value.get_self_ref()
    }
    fn get_pool_ref(&self) -> Self::TPoolId {
        self.value.get_pool_ref()
    }
}

#[derive(Debug, Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Hash, derive_more::From, derive_more::Into)]
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

impl TryFrom<TaggedAssetClass<PoolNft>> for PoolId {
    type Error = ();
    fn try_from(value: TaggedAssetClass<PoolNft>) -> Result<Self, Self::Error> {
        Ok(PoolId(AssetClass::from(value).into_token().ok_or(())?))
    }
}

#[repr(transparent)]
#[derive(Debug, Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Hash, derive_more::From, derive_more::Into)]
pub struct PoolStateVer(OutputRef);

#[derive(Debug, Clone)]
pub struct ExecutorFeePerToken(Ratio<u64>, AssetClass);

impl ExecutorFeePerToken {
    pub fn new(rational: Ratio<u64>, ac: AssetClass) -> Self {
        Self(rational, ac)
    }
    pub fn get_fee(&self, quote_amount: u64) -> u64 {
        ((*self.0.numer() as u128) * (quote_amount as u128) / (*self.0.denom() as u128)) as u64
    }
}
