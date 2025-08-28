use cml_chain::certs::Credential;
use cml_core::serialization::{Deserialize, Serialize};
use cml_core::Slot;
use rocksdb::{
    ColumnFamily, DBCommon, DBIteratorWithThreadMode, Direction, IteratorMode, Options, ReadOptions,
    SingleThreaded, SnapshotWithThreadMode, Transaction, TransactionDB, TransactionDBOptions,
};
use serde::de::DeserializeOwned;
use spectrum_offchain_cardano::data::PoolId;
use splash_yf_offchain::ve_config::VeConfig;
use splash_yf_offchain::Epoch;
use std::io::Read;
use std::mem::size_of;
use std::path::Path;
use std::sync::Arc;

pub mod accounts;
pub mod event_log;
pub mod export_feed;
pub mod mature_events;

#[derive(Clone)]
pub struct PositionDB {
    pub db: Arc<TransactionDB>,
    pub confirmation_delay_slots: u64,
    pub ve_config: VeConfig,
}

impl PositionDB {
    pub fn new<P: AsRef<Path>>(db_path: P, confirmation_delay_slots: u64, ve_config: VeConfig) -> Self {
        let mut opts = Options::default();
        opts.create_if_missing(true);
        opts.create_missing_column_families(true);
        let db_opts = TransactionDBOptions::default();
        Self {
            db: Arc::new(TransactionDB::open_cf(&opts, &db_opts, db_path, COLUMN_FAMILIES).unwrap()),
            confirmation_delay_slots,
            ve_config,
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

pub(crate) fn get_range_iterator_over_snapshot<'a: 'b, 'b>(
    db: &'a SnapshotWithThreadMode<'a, TransactionDB>,
    cf: &ColumnFamily,
    prefix: Vec<u8>,
    start_from_key: Vec<u8>,
) -> DBIteratorWithThreadMode<'b, TransactionDB> {
    let mut readopts = ReadOptions::default();
    readopts.set_iterate_range(rocksdb::PrefixRange(prefix));
    db.iterator_cf_opt(
        cf,
        readopts,
        IteratorMode::From(&start_from_key, Direction::Forward),
    )
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

pub(crate) fn position_key(pool_id: PoolId, credential: &Credential, epoch: Epoch) -> Vec<u8> {
    let mut key: Vec<u8> = pool_id.into();
    key.extend(credential.to_canonical_cbor_bytes());
    key.extend(epoch.unwrap().to_be_bytes());
    key
}

pub(crate) fn account_positions_key(pool_id: PoolId, credential: &Credential) -> Vec<u8> {
    let mut key: Vec<u8> = pool_id.into();
    key.extend(credential.to_canonical_cbor_bytes());
    key
}

pub(crate) fn parse_position_key(mut key: Vec<u8>) -> Option<(PoolId, Credential, Epoch)> {
    PoolId::try_from(&key[..PoolId::BYTE_COUNT])
        .ok()
        .and_then(|pool_id| {
            let slot_position = key.len() - 8;
            Credential::from_cbor_bytes(&key[PoolId::BYTE_COUNT..slot_position])
                .ok()
                .and_then(|cred| {
                    let epoch = Slot::from_be_bytes(key[slot_position..].try_into().ok()?);
                    Some((pool_id, cred, epoch.into()))
                })
        })
}

pub(crate) fn gauge_key(pool_id: PoolId, epoch: Epoch) -> Vec<u8> {
    let mut key: Vec<u8> = pool_id.into();
    key.extend(epoch.unwrap().to_be_bytes());
    key
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

pub(crate) fn account_to_pools_index(credential: &Credential, pool_id: PoolId) -> Vec<u8> {
    rmp_serde::to_vec(&(credential, pool_id)).unwrap()
}

pub(crate) fn account_to_pools_index_prefix(credential: &Credential) -> Vec<u8> {
    let mut prefix: Vec<u8> = vec![TUPLE_PREFIX];
    prefix.extend(rmp_serde::to_vec(&credential).unwrap());
    prefix
}

pub(crate) fn parse_account_to_pools_index(key: Vec<u8>) -> Option<(Credential, PoolId)> {
    rmp_serde::from_slice(&key).ok()
}

const TUPLE_PREFIX: u8 = 0x92;

// Unconfirmed LP events
// key: [slot:index], value: [event]
pub(crate) const EVENTS_CF: &str = "events";

// Accounts
// key: [pool_id:credential:epoch], value: [account_position]
pub(crate) const ACCOUNT_POSITIONS_CF: &str = "account_positions";

// Accounts to pools mapping
// key: [credential:pool_id], value: []
pub(crate) const ACCOUNT_POOLS_CF: &str = "account_pools";

// Gauge weights
// key: [pool_id:epoch], value: [gauge_weight]
pub(crate) const GAUGE_WEIGHTS_CF: &str = "gauges";

// LQ supply by pool
// key: [pool_id], value: [lq_supply]
pub(crate) const POOL_LQ_CF: &str = "pools";

// Key-value store for other stuff
pub(crate) const KV_CF: &str = "aggregates";

pub(crate) const ACCOUNT_FEED_EXPORT_CF: &str = "account_feed_export";

pub(crate) const CURRENT_SLOT_KEY: [u8; 4] = [0u8; 4];

pub(crate) fn get_current_slot(db: &Transaction<TransactionDB>, cf: &ColumnFamily) -> Option<Slot> {
    db.get_cf(cf, CURRENT_SLOT_KEY)
        .unwrap()
        .map(|raw| rmp_serde::from_slice::<u64>(&raw).unwrap())
}

pub(crate) const COLUMN_FAMILIES: [&str; 7] = [
    EVENTS_CF,
    ACCOUNT_POSITIONS_CF,
    ACCOUNT_POOLS_CF,
    ACCOUNT_FEED_EXPORT_CF,
    GAUGE_WEIGHTS_CF,
    POOL_LQ_CF,
    KV_CF,
];

pub(crate) struct ColumnFamilies<'a> {
    pub events: &'a ColumnFamily,
    pub account_positions: &'a ColumnFamily,
    pub account_pools: &'a ColumnFamily,
    pub account_feed_export: &'a ColumnFamily,
    pub gauge_weights: &'a ColumnFamily,
    pub pool_lq: &'a ColumnFamily,
    pub kv: &'a ColumnFamily,
}

impl<'a> ColumnFamilies<'a> {
    pub(crate) fn new(db: &'a Arc<TransactionDB>) -> Self {
        ColumnFamilies {
            events: db.cf_handle(EVENTS_CF).unwrap(),
            account_positions: db.cf_handle(ACCOUNT_POSITIONS_CF).unwrap(),
            account_pools: db.cf_handle(ACCOUNT_POOLS_CF).unwrap(),
            account_feed_export: db.cf_handle(ACCOUNT_FEED_EXPORT_CF).unwrap(),
            gauge_weights: db.cf_handle(GAUGE_WEIGHTS_CF).unwrap(),
            pool_lq: db.cf_handle(POOL_LQ_CF).unwrap(),
            kv: db.cf_handle(KV_CF).unwrap(),
        }
    }
}

#[cfg(test)]
pub mod tests {
    use crate::position_db::{account_to_pools_index, account_to_pools_index_prefix, read_max_key};
    use cml_chain::certs::Credential;
    use cml_crypto::Ed25519KeyHash;
    use rand::RngCore;
    use rocksdb::{Options, SingleThreaded, TransactionDB, TransactionDBOptions};
    use spectrum_offchain_cardano::data::PoolId;
    use splash_testing::db_path::DBPath;
    use std::sync::Arc;

    #[test]
    fn credential_keys_test() {
        let mut bf = [0u8; 28];
        rand::thread_rng().fill_bytes(&mut bf);

        let random_cred_bytes = Ed25519KeyHash::from(bf);
        let random_credential = Credential::new_pub_key(random_cred_bytes);
        let random_pool_id = PoolId::random();

        let credential_index_key = account_to_pools_index(&random_credential, random_pool_id);

        let cred_index_prefix = account_to_pools_index_prefix(&random_credential);

        let cred_index_prefix_is_correct = credential_index_key.starts_with(cred_index_prefix.as_ref());

        assert!(cred_index_prefix_is_correct);
    }

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
