use cml_chain::certs::Credential;
use cml_core::serialization::{Deserialize, Serialize, ToBytes};
use cml_core::Slot;
use rocksdb::{
    ColumnFamily, DBIteratorWithThreadMode, Direction, IteratorMode, Options, ReadOptions, Transaction,
    TransactionDB, TransactionDBOptions,
};
use serde::de::DeserializeOwned;
use spectrum_offchain_cardano::data::PoolId;
use std::path::Path;
use std::sync::Arc;

pub mod accounts;
pub mod event_log;
pub mod export_feed;
pub mod mature_events;
pub mod pool_frames;

#[derive(Clone)]
pub struct PositionDB {
    pub db: Arc<TransactionDB>,
}

impl PositionDB {
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

pub(crate) fn read_min_kv<T: DeserializeOwned>(
    db: &Arc<TransactionDB>,
    cf: &ColumnFamily,
) -> Option<(u64, T)> {
    let readopts = ReadOptions::default();
    let mut iter = db.iterator_cf_opt(cf, readopts, IteratorMode::Start);
    if let Some(Ok((key, value))) = iter.next() {
        let max_seq_num = rmp_serde::from_slice(&key).unwrap();
        return Some((max_seq_num, rmp_serde::from_slice(&value).unwrap()));
    }
    None
}

pub(crate) fn pool_key(pool_id: PoolId) -> Vec<u8> {
    pool_id.into()
}

pub(crate) fn account_key(pool_id: PoolId, credential: Credential) -> Vec<u8> {
    let mut key: Vec<u8> = pool_id.into();
    key.extend(credential.to_canonical_cbor_bytes());
    key
}

pub(crate) fn from_account_key(key: Vec<u8>) -> Option<(PoolId, Credential)> {
    PoolId::try_from(&key[..PoolId::BYTE_COUNT])
        .ok()
        .and_then(|pool_id| {
            Credential::from_cbor_bytes(&key[PoolId::BYTE_COUNT..])
                .ok()
                .map(|cred| (pool_id, cred))
        })
}

pub(crate) fn event_key(slot: Slot, event_index: usize) -> Vec<u8> {
    let mut event_key: Vec<u8> = slot.to_be_bytes().to_vec();
    event_key.extend(event_index.to_be_bytes());
    event_key
}

pub(crate) fn from_event_key(key: Vec<u8>) -> Option<(Slot, usize)> {
    if key.len() >= size_of::<Slot>() {
        let (slot_bytes, event_index_bytes) = key.split_at(size_of::<Slot>());
        let slot = Slot::from_be_bytes(slot_bytes.try_into().ok()?);
        let event_index = usize::from_be_bytes(event_index_bytes.try_into().ok()?);
        return Some((slot, event_index));
    }
    None
}

pub(crate) fn sus_event_key(cred: Credential, slot: Slot) -> Vec<u8> {
    rmp_serde::to_vec(&(cred, slot)).unwrap()
}

pub(crate) fn from_sus_event_key(key: Vec<u8>) -> Option<(Credential, Slot)> {
    rmp_serde::from_slice(&key).ok()
}

pub(crate) fn cred_index_key(credential: &Credential, pool_id: PoolId) -> Vec<u8> {
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

pub(crate) const POOL_LQ_FRAMES_INDEX_CF: &str = "pool_frames_index";

pub(crate) const ACCOUNT_FEED_CF: &str = "account_events";

pub(crate) const MAX_BLOCK_NUM_KEY: [u8; 4] = [0u8; 4];

pub(crate) const COLUMN_FAMILIES: [&str; 8] = [
    EVENTS_CF,
    ACCOUNTS_CF,
    ACTIVE_FARMS_CF,
    AGGREGATE_CF,
    SUS_EVENTS_CF,
    CREDS_INDEX_CF,
    ACCOUNT_FEED_CF,
    POOL_LQ_FRAMES_INDEX_CF
];

#[cfg(test)]
mod tests {
    use crate::position_db::read_max_key;
    use rocksdb::{Options, SingleThreaded, TransactionDB, TransactionDBOptions};
    use std::sync::Arc;
    use splash_testing::db_path::DBPath;

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
}
