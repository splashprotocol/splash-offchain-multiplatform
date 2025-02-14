use cml_chain::certs::Credential;
use cml_core::Slot;
use rocksdb::{
    ColumnFamily, DBIteratorWithThreadMode, Direction, IteratorMode, Options, ReadOptions, Transaction,
    TransactionDB, TransactionDBOptions,
};
use spectrum_offchain_cardano::data::PoolId;
use std::path::Path;
use std::sync::Arc;

mod account_feed;
pub mod accounts;
pub mod event_log;
pub mod mature_events;

#[derive(Clone)]
pub struct RocksDB {
    pub db: Arc<TransactionDB>,
}

impl RocksDB {
    pub fn new<P: AsRef<Path>>(db_path: P) -> Self {
        let mut opts = Options::default();
        opts.create_if_missing(true);
        opts.create_missing_column_families(true);
        let db_opts = TransactionDBOptions::default();
        Self {
            db: Arc::new(TransactionDB::open_cf(&opts, &db_opts, db_path, COLUMN_FAMILIES).unwrap()),
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

pub(crate) fn read_max_key(tx: &Transaction<TransactionDB>, cf: &ColumnFamily) -> u64 {
    let readopts = ReadOptions::default();
    let mut iter = tx.iterator_cf_opt(cf, readopts, IteratorMode::End);
    let mut seq_num = 0u64;
    if let Some(Ok((key, _))) = iter.next() {
        let max_seq_num = rmp_serde::from_slice(&key).unwrap();
        seq_num = max_seq_num;
    }
    seq_num
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

pub(crate) const ACCOUNT_FEED_CF: &str = "account_events";

pub(crate) const MAX_BLOCK_KEY: [u8; 4] = [0u8; 4];

pub(crate) const COLUMN_FAMILIES: [&str; 6] = [
    EVENTS_CF,
    ACCOUNTS_CF,
    AGGREGATE_CF,
    SUS_EVENTS_CF,
    CREDS_INDEX_CF,
    ACCOUNT_FEED_CF,
];

#[cfg(test)]
mod tests {
    use crate::db::read_max_key;
    use rocksdb::{Options, SingleThreaded, TransactionDB, TransactionDBOptions};
    use std::path::{Path, PathBuf};
    use std::sync::Arc;

    #[test]
    fn test_read_max_key() {
        let n = DBPath::new("_test_read_max_key");
        let mut opts = Options::default();
        opts.create_if_missing(true);
        opts.create_missing_column_families(true);
        let db_opts = TransactionDBOptions::default();
        let cf = "test_cf";
        let db = Arc::new(TransactionDB::<SingleThreaded>::open_cf(&opts, &db_opts, &n, [cf]).unwrap());
        let cf = db.cf_handle(cf).unwrap();
        let kvs = vec![1, 2, 3, 4, 128, 1024]
            .into_iter()
            .map(|i| (rmp_serde::to_vec(&i).unwrap(), vec![0u8]));
        let tx = db.transaction();
        for (k, v) in kvs {
            tx.put_cf(cf, k, v).unwrap();
        }
        let max_key = read_max_key(&tx, cf);
        assert_eq!(max_key, 1024);
    }

    /// Temporary database path which calls DB::Destroy when DBPath is dropped.
    pub struct DBPath {
        dir: tempfile::TempDir, // kept for cleaning up during drop
        path: PathBuf,
    }

    impl DBPath {
        /// Produces a fresh (non-existent) temporary path which will be DB::destroy'ed automatically.
        pub fn new(prefix: &str) -> DBPath {
            let dir = tempfile::Builder::new()
                .prefix(prefix)
                .tempdir()
                .expect("Failed to create temporary path for db.");
            let path = dir.path().join("db");

            DBPath { dir, path }
        }
    }

    impl Drop for DBPath {
        fn drop(&mut self) {
            let opts = Options::default();
            TransactionDB::<SingleThreaded>::destroy(&opts, &self.path)
                .expect("Failed to destroy temporary DB");
        }
    }

    /// Convert a DBPath ref to a Path ref.
    /// We don't implement this for DBPath values because we want them to
    /// exist until the end of their scope, not get passed into functions and
    /// dropped early.
    impl AsRef<Path> for &DBPath {
        fn as_ref(&self) -> &Path {
            &self.path
        }
    }
}
