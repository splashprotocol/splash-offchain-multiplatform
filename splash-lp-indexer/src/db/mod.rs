use cml_chain::certs::Credential;
use cml_core::Slot;
use rocksdb::{
    ColumnFamily, DBIteratorWithThreadMode, Direction, IteratorMode, Options, ReadOptions, TransactionDB,
    TransactionDBOptions,
};
use spectrum_offchain_cardano::data::PoolId;
use std::string::ToString;
use std::sync::Arc;

mod accounts;
pub mod event_log;
pub mod mature_events;

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

pub(crate) fn get_range_iterator<'a: 'b, 'b>(
    db: &'a Arc<TransactionDB>,
    cf: &ColumnFamily,
    prefix: Vec<u8>,
) -> DBIteratorWithThreadMode<'b, TransactionDB> {
    let mut readopts = ReadOptions::default();
    readopts.set_iterate_range(rocksdb::PrefixRange(prefix.clone()));
    db.iterator_cf_opt(cf, readopts, IteratorMode::From(&prefix, Direction::Forward))
}

pub(crate) fn account_key(pool_id: PoolId, credential: Credential) -> Vec<u8> {
    rmp_serde::to_vec(&(pool_id, credential)).unwrap()
}

pub(crate) fn from_account_key(key: Vec<u8>) -> Option<(PoolId, Credential)> {
    rmp_serde::from_slice(&key).ok()
}

pub(crate) fn event_key(slot: Slot, event_index: usize) -> Vec<u8> {
    rmp_serde::to_vec(&(slot, event_index)).unwrap()
}

pub(crate) fn from_event_key(key: Vec<u8>) -> Option<(Slot, usize)> {
    rmp_serde::from_slice(&key).ok()
}

pub(crate) fn sus_event_key(cred: Credential, slot: Slot) -> Vec<u8> {
    rmp_serde::to_vec(&(cred, slot)).unwrap()
}

pub(crate) fn from_sus_event_key(key: Vec<u8>) -> Option<(Credential, Slot)> {
    rmp_serde::from_slice(&key).ok()
}

pub(crate) fn cred_index_key(credential: Credential, pool_id: PoolId) -> Vec<u8> {
    rmp_serde::to_vec(&(credential, pool_id)).unwrap()
}

pub(crate) fn cred_index_prefix(credential: Credential) -> Vec<u8> {
    rmp_serde::to_vec(&credential).unwrap()
}

pub(crate) fn from_cred_index_key(key: Vec<u8>) -> Option<(Credential, PoolId)> {
    rmp_serde::from_slice(&key).ok()
}

// Unconfirmed LP events
pub(crate) const EVENTS_CF: &str = "events";

// Accounts
pub(crate) const ACCOUNTS_CF: &str = "accounts";

// Active farms
pub(crate) const ACTIVE_FARMS_CF: &str = "farms";

// Aggregate data
pub(crate) const AGGREGATE_CF: &str = "aggregates";

pub(crate) const SUS_EVENTS_CF: &str = "sus_events";

pub(crate) const CREDS_INDEX_CF: &str = "creds_index";

pub(crate) const MAX_BLOCK_KEY: [u8; 4] = [0u8; 4];
