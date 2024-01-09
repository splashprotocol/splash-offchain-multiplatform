use std::fmt::{Display, Formatter};

use cml_chain::address::Address;
use cml_chain::transaction::{TransactionInput, TransactionOutput};
use cml_chain::PolicyId;
use cml_crypto::{RawBytesEncoding, TransactionHash};
use cml_multi_era::babbage::BabbageTransactionOutput;
use num_rational::Ratio;
use rand::{thread_rng, RngCore};

use spectrum_cardano_lib::transaction::BabbageTransactionOutputExtension;
use spectrum_cardano_lib::{AssetClass, AssetName, OutputRef, TaggedAssetClass, Token};
use spectrum_offchain::data::order::SpecializedOrder;
use spectrum_offchain::data::{EntitySnapshot, Stable};
use spectrum_offchain::ledger::TryFromLedger;

use crate::constants::POOL_VERSIONS;
use crate::data::order::PoolNft;

pub mod deposit;
pub mod limit_swap;
pub mod operation_output;
pub mod order;
pub mod pool;
pub mod redeem;

pub mod ref_scripts;

pub mod execution_context;
pub mod pair;

/// For persistent on-chain entities (e.g. pools) we want to carry initial utxo.
#[derive(Debug, Clone)]
pub struct OnChain<T> {
    pub value: T,
    pub source: TransactionOutput,
}

impl<T, Ctx> TryFromLedger<TransactionOutput, Ctx> for OnChain<T>
where
    T: TryFromLedger<TransactionOutput, Ctx>,
{
    fn try_from_ledger(repr: &TransactionOutput, ctx: Ctx) -> Option<Self> {
        T::try_from_ledger(repr, ctx).map(|value| OnChain {
            value,
            source: repr.clone(),
        })
    }
}

impl<T, Ctx> TryFromLedger<BabbageTransactionOutput, Ctx> for OnChain<T>
where
    T: TryFromLedger<BabbageTransactionOutput, Ctx>,
{
    fn try_from_ledger(repr: &BabbageTransactionOutput, ctx: Ctx) -> Option<Self> {
        T::try_from_ledger(repr, ctx).map(|value| OnChain {
            value,
            source: repr.clone().upcast(),
        })
    }
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

impl<T> Stable for OnChain<T>
where
    T: Stable,
{
    type StableId = T::StableId;

    fn stable_id(&self) -> Self::StableId {
        self.value.stable_id()
    }
}

impl<T> EntitySnapshot for OnChain<T>
where
    T: EntitySnapshot,
{
    type Version = T::Version;

    fn version(&self) -> Self::Version {
        self.value.version()
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
    pub fn get_fee(&self, output_amount: u64) -> u64 {
        ((*self.0.numer() as u128) * (output_amount as u128) / (*self.0.denom())) as u64
    }
    pub fn value(&self) -> Ratio<u128> {
        self.0
    }
}

#[repr(transparent)]
#[derive(Debug, Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Hash, derive_more::From, derive_more::Into)]
pub struct PoolVer(pub u8);

impl PoolVer {
    pub const V1: PoolVer = PoolVer(1);
    pub const V2: PoolVer = PoolVer(2);

    pub fn try_from_pool_address(pool_addr: &Address) -> Option<PoolVer> {
        let this_addr = pool_addr.to_bech32(None).unwrap();
        POOL_VERSIONS.iter().find_map(|(addr, v)| {
            if this_addr == *addr {
                Some(PoolVer(*v))
            } else {
                None
            }
        })
    }
}
