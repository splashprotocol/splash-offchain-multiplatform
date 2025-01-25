use rocksdb::{Options, TransactionDBOptions};
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

fn read_block_num_from_key_unsafe(key: Vec<u8>) -> u64 {
    <u64>::from_be_bytes(<[u8; 8]>::try_from(&key[..8]).unwrap())
}

fn read_block_num_unsafe(key: Vec<u8>) -> u64 {
    <u64>::from_be_bytes(key.as_slice().try_into().unwrap())
}

// Unconfirmed LP events
pub(crate) const EVENTS_CF: &str = "events";

// Accounts
pub(crate) const ACCOUNTS_CF: &str = "accounts";

// Aggregate data
pub(crate) const AGGREGATE_CF: &str = "aggregates";

pub(crate) const MAX_BLOCK_KEY: [u8; 4] = [0u8; 4];
