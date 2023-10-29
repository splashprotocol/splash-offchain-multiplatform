use cml_multi_era::babbage::BabbageTransactionOutput;
use num_rational::Ratio;

use spectrum_cardano_lib::address::AddressExtension;
use spectrum_cardano_lib::transaction::TransactionOutputExtension;
use spectrum_cardano_lib::{OutputRef, TaggedAmount, TaggedAssetClass};
use spectrum_offchain::data::OnChainEntity;
use spectrum_offchain::ledger::TryFromLedger;

use crate::amm::{minswap, Platform, PoolId, PoolVersion};

/// Asset X
pub struct Rx;
/// Asset Y
pub struct Ry;

/// Snapshot of a CFMM pool.
#[derive(Debug, Clone)]
pub struct CFMMPool {
    /// Unique identifier of the pool.
    pub id: PoolId,
    /// Version of the pool state.
    pub version: PoolVersion,
    /// Platform the pool belongs to.
    pub dex: Platform,
    pub reserves_x: TaggedAmount<Rx>,
    pub reserves_y: TaggedAmount<Ry>,
    pub asset_x: TaggedAssetClass<Rx>,
    pub asset_y: TaggedAssetClass<Ry>,
    pub lp_fee: Ratio<u64>,
}

impl OnChainEntity for CFMMPool {
    type Id = PoolId;
    type Version = PoolVersion;

    fn get_id(&self) -> Self::Id {
        self.id
    }

    fn get_version(&self) -> Self::Version {
        self.version
    }
}

impl TryFromLedger<BabbageTransactionOutput, OutputRef> for CFMMPool {
    fn try_from_ledger(repr: BabbageTransactionOutput, ctx: OutputRef) -> Option<Self> {
        if repr.address().script_hash().is_none() || repr.datum().is_none() {
            None
        } else {
            minswap::parsers::pool_from_utxo(repr, ctx)
        }
    }
}
