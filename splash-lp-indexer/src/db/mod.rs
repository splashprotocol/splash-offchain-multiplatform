use cml_chain::certs::Credential;
use cml_core::Slot;
use rocksdb::{Options, TransactionDBOptions};
use spectrum_offchain_cardano::data::PoolId;
use std::string::ToString;
use std::sync::Arc;

pub mod accounts;
pub mod event_log;

#[derive(Clone)]
pub struct RocksDB {
    pub db: Arc<rocksdb::TransactionDB>,
}

impl RocksDB {
    pub fn new(db_path: String) -> Self {
        let opts = Options::default();
        let db_opts = TransactionDBOptions::default();
        let cfs = vec![EVENTS_CF, ACCOUNTS_CF, AGGREGATE_CF];
        Self {
            db: Arc::new(rocksdb::TransactionDB::open_cf(&opts, &db_opts, db_path, cfs).unwrap()),
        }
    }
}

fn account_key(pool_id: PoolId, credential: Credential) -> Vec<u8> {
    rmp_serde::to_vec(&(pool_id, credential)).unwrap()
}

fn from_account_key(key: Vec<u8>) -> Option<(PoolId, Credential)> {
    rmp_serde::from_slice(&key).ok()
}

fn event_key(slot: Slot, event_index: usize) -> Vec<u8> {
    rmp_serde::to_vec(&(slot, event_index)).unwrap()
}

fn from_event_key(key: Vec<u8>) -> Option<(Slot, usize)> {
    rmp_serde::from_slice(&key).ok()
}

// Unconfirmed LP events
pub(crate) const EVENTS_CF: &str = "events";

// Accounts
pub(crate) const ACCOUNTS_CF: &str = "accounts";

// Aggregate data
pub(crate) const AGGREGATE_CF: &str = "aggregates";

pub(crate) const MAX_BLOCK_KEY: [u8; 4] = [0u8; 4];
