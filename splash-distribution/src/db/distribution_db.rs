use crate::entities::smart_farm::{DistributorSmartFarmSnapshot, SmartFarm, SmartFarmStatus};
use crate::index::status_events_index::StatusEntitiesIndex;
use crate::smart_farm_holder::SmartFarmHolder;
use async_trait::async_trait;
use bloom_offchain::execution_engine::bundled::Bundled;
use bloom_offchain::execution_engine::liquidity_book::core::Unit;
use cml_chain::assets::MultiAsset;
use cml_chain::transaction::TransactionOutput;
use cml_chain::Value;
use cml_crypto::blake2b224;
use log::info;
use rocksdb::{
    ColumnFamily, DBIteratorWithThreadMode, Direction, IteratorMode, OptimisticTransactionDB, Options,
    ReadOptions, SnapshotWithThreadMode, TransactionDB, TransactionDBOptions,
};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use spectrum_cardano_lib::output::FinalizedTxOut;
use spectrum_cardano_lib::transaction::TransactionOutputExtension;
use spectrum_cardano_lib::OutputRef;
use spectrum_offchain::domain::Stable;
use splash_dao_offchain::entities::onchain::smart_farm::SmartFarmSnapshot;
use splash_dao_offchain::entities::{HasStatus, Snapshot};
use splash_dao_offchain::routines::TimedOutputRef;
use std::fmt::Debug;
use std::path::Path;
use std::sync::Arc;
use tokio::task::spawn_blocking;

#[derive(Clone)]
pub struct DistributionDb {
    pub db: Arc<rocksdb::OptimisticTransactionDB>,
}
impl DistributionDb {
    pub fn new<P>(db_path: P) -> Self
    where
        P: AsRef<Path>,
    {
        let mut opts = Options::default();
        opts.create_if_missing(true);
        opts.create_missing_column_families(true);
        Self {
            db: Arc::new(rocksdb::OptimisticTransactionDB::open_cf(&opts, db_path, TABLES).unwrap()),
        }
    }

    pub(crate) fn get_range_iterator<'a: 'b, 'b>(
        db: &'a SnapshotWithThreadMode<'b, OptimisticTransactionDB>,
        cf: &ColumnFamily,
        prefix: Vec<u8>,
    ) -> DBIteratorWithThreadMode<'b, OptimisticTransactionDB> {
        let mut readopts = ReadOptions::default();
        readopts.set_iterate_range(rocksdb::PrefixRange(prefix.clone()));
        db.iterator_cf_opt(cf, readopts, IteratorMode::From(&prefix, Direction::Forward))
    }
}

const TABLES: [&str; 2] = ["statuses", "entities"];

#[async_trait]
impl<K, V> StatusEntitiesIndex<K, V> for DistributionDb
where
    K: Serialize + DeserializeOwned + Send + 'static + Clone,
    V: HasStatus + Serialize + DeserializeOwned + 'static + Send,
    <V as HasStatus>::Status: Debug + Send + Serialize + DeserializeOwned,
{
    fn status_entity_key(key: K, entity: Bundled<V, FinalizedTxOut>) -> Vec<u8> {
        let raw_status = rmp_serde::to_vec_named(&entity.0.get_status()).unwrap();
        let mut wrapped_status = [0u8; 32];
        wrapped_status[..raw_status.len()].copy_from_slice(&raw_status);

        let mut common_key: Vec<u8> = vec![];
        common_key.extend(wrapped_status);
        common_key.extend(rmp_serde::to_vec_named(&key).unwrap());
        common_key
    }

    // todo: add correlation between status and cf
    async fn get_events_by_status(
        &self,
        status: <V as HasStatus>::Status,
    ) -> Vec<Bundled<V, FinalizedTxOut>> {
        let db = self.db.clone();

        spawn_blocking(move || {
            let tx = db.transaction();
            let statuses_cf = db.cf_handle(TABLES[0]).unwrap();
            let snap = db.snapshot();

            let raw_status = rmp_serde::to_vec_named(&status).unwrap();
            let mut wrapped_status = [0u8; 32];
            wrapped_status[..raw_status.len()].copy_from_slice(&raw_status);

            let mut new_events =
                DistributionDb::get_range_iterator(&snap, statuses_cf, wrapped_status.to_vec());
            tx.commit().unwrap();
            let mut utxo_set: Vec<Bundled<V, FinalizedTxOut>> = vec![];
            while let Some(Ok(bytes)) = new_events.next() {
                if let Ok(Some(new_event)) = rmp_serde::from_slice(&bytes.1.to_vec()) {
                    utxo_set.push(new_event);
                }
            }
            utxo_set
        })
        .await
        .unwrap()
    }

    async fn get_event_by_key(&self, key: K) -> Option<Bundled<V, FinalizedTxOut>> {
        let db = self.db.clone();
        spawn_blocking(move || {
            let entities_cf = db.cf_handle(&TABLES[1]).unwrap();
            let db_get_result = db.get_cf(entities_cf, rmp_serde::to_vec_named(&key).unwrap());
            if let Ok(Some(raw_value)) = db.get_cf(entities_cf, rmp_serde::to_vec_named(&key).unwrap()) {
                let res = rmp_serde::from_slice(&raw_value);
                res.ok()
            } else {
                None
            }
        })
        .await
        .unwrap()
    }

    async fn drop_event(&self, event: K) {
        let db = self.db.clone();
        spawn_blocking(move || {
            let tx = db.transaction();
            let new_events = db.cf_handle(TABLES[1]).unwrap();
            let encoded_value = serde_json::to_string(&event).unwrap();
            let key = blake2b224(encoded_value.as_bytes());
            tx.delete_cf(new_events, key).unwrap();
            tx.commit().unwrap();
        })
        .await
        .unwrap();
    }

    async fn put(&self, key: K, event: Bundled<V, FinalizedTxOut>) {
        let db = self.db.clone();
        info!("put");
        spawn_blocking(move || {
            let tx = db.transaction();

            let statuses_cf = db.cf_handle(TABLES[0]).unwrap();

            let events_cf = db.cf_handle(TABLES[1]).unwrap();

            let encoded_value = rmp_serde::to_vec_named(&event).unwrap();

            let event_key = rmp_serde::to_vec_named(&key).unwrap();

            let status_event_key = Self::status_entity_key(key, event);

            let empty_vec: Vec<u8> = vec![];
            tx.put_cf(events_cf, event_key.clone(), encoded_value).unwrap();

            tx.put_cf(statuses_cf, status_event_key.clone(), empty_vec)
                .unwrap();

            tx.commit().unwrap();
        })
        .await
        .unwrap();
    }

    async fn update(&self, key: K, event: Bundled<V, FinalizedTxOut>) {
        let db = self.db.clone();
        spawn_blocking(move || {
            let tx = db.transaction();
            let raw_key = rmp_serde::to_vec_named(&key.clone()).unwrap();

            let statuses_cf = db.cf_handle(TABLES[0]).unwrap();

            let events_cf = db.cf_handle(TABLES[1]).unwrap();

            if let Ok(Some(raw_value)) = tx.get_for_update(raw_key.clone(), true) {

                let prev_value: Bundled<V, FinalizedTxOut> = rmp_serde::from_slice(&raw_value).unwrap();

                let prev_status_key = Self::status_entity_key(key.clone(), prev_value);

                tx.delete_cf(statuses_cf, prev_status_key).unwrap();

                tx.delete_cf(events_cf, &raw_key).unwrap();

                let new_encoded_value = rmp_serde::to_vec_named(&event).unwrap();

                let new_status_key = Self::status_entity_key(key.clone(), event);

                let empty_vec = &vec![];
                tx.put_cf(statuses_cf, new_status_key, empty_vec).unwrap();

                tx.put_cf(events_cf, rmp_serde::to_vec_named(&key).unwrap(), new_encoded_value)
                    .unwrap();
            }
            tx.commit().unwrap();
        })
        .await
        .unwrap();
    }

    async fn update_event_status(&self, event_key: K, status: <V as HasStatus>::Status) {
        todo!()
    }
}

#[async_trait]
impl SmartFarmHolder for DistributionDb {
    async fn get_free_farms_by_value(
        &self,
        value: Value,
    ) -> Vec<Bundled<DistributorSmartFarmSnapshot, TransactionOutput>> {
        let db = self.db.clone();
        spawn_blocking(move || {
            let tx = db.transaction();

            let mut smart_farm_to_return: Vec<Bundled<DistributorSmartFarmSnapshot, TransactionOutput>> =
                vec![];

            let new_events = db.cf_handle(TABLES[0]).unwrap();
            let readopts = ReadOptions::default();

            let mut iterator = db.iterator_cf_opt(new_events, readopts, IteratorMode::Start);

            let acc_value = Value::new(0, MultiAsset::new());

            while let Some(Ok((_, raw_entity_value))) = iterator.next() {
                let smart_farm: Bundled<DistributorSmartFarmSnapshot, TransactionOutput> =
                    rmp_serde::from_slice(&raw_entity_value).unwrap();
                if acc_value.coin > value.coin && acc_value.multiasset > value.multiasset {
                    continue;
                } else {
                    acc_value.checked_add(smart_farm.clone().1.value()).unwrap();
                    smart_farm_to_return.push(smart_farm)
                }
            }

            tx.commit().unwrap();

            smart_farm_to_return
        })
        .await
        .unwrap_or(vec![])
    }

    async fn update_smart_farms_statuses(
        &self,
        farms: Vec<DistributorSmartFarmSnapshot>,
        status: SmartFarmStatus,
    ) {
        let db = self.db.clone();
        spawn_blocking(move || {
            let tx = db.transaction();

            let new_events = db.cf_handle(TABLES[0]).unwrap();

            for farm in farms {
                if let Ok(Some(farm_entity)) =
                    tx.get_for_update(rmp_serde::to_vec_named(&farm.clone().unwrap().id).unwrap(), true)
                {
                    let mut smart_farm: Bundled<DistributorSmartFarmSnapshot, FinalizedTxOut> =
                        rmp_serde::from_slice(&farm_entity).unwrap();
                    let mut sf: SmartFarm = smart_farm.0.clone().unwrap();
                    sf.status = status.clone();
                    let new_snapshot: Snapshot<SmartFarm, OutputRef> =
                        Snapshot::new(sf, smart_farm.0.version().clone());
                    let new_bundled = Bundled(new_snapshot, smart_farm.1.clone());
                    tx.put_cf(
                        new_events,
                        rmp_serde::to_vec_named(&farm.unwrap().id).unwrap(),
                        rmp_serde::to_vec_named(&new_bundled).unwrap(),
                    )
                    .unwrap()
                }
            }
            tx.commit().unwrap();
        })
        .await
        .unwrap()
    }
}
