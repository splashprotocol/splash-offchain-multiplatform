use std::{path::Path, sync::Arc};

use async_std::task::spawn_blocking;
use async_trait::async_trait;
use log::trace;
use rocksdb::{Direction, IteratorMode, ReadOptions};
use spectrum_offchain::domain::event::{Confirmed, Predicted};

use crate::entities::onchain::funding_box::{FundingBox, FundingBoxId};

#[async_trait]
pub trait FundingRepo {
    /// Collect funding boxes that cover the specified `target`.
    async fn collect(&self) -> Result<AvailableFundingBoxes, ()>;
    async fn put_confirmed(&self, f: Confirmed<FundingBox>);
    async fn put_predicted(&self, f: Predicted<FundingBox>);
    async fn spend_confirmed(&self, f_id: FundingBoxId);
    async fn unspend_confirmed(&self, f_id: FundingBoxId);
    async fn spend_predicted(&self, f_id: FundingBoxId);
    async fn unspend_predicted(&self, f_id: FundingBoxId);
    async fn eliminate_predicted(&self, f_id: FundingBoxId);
    async fn eliminate_confirmed(&self, f_id: FundingBoxId);
}

const STATE_PREFIX: &str = "s:";
const CONFIRMED_AVAILABLE: u8 = 0;
const PREDICTED_AVAILABLE: u8 = 5;
const CONFIRMED_SPENT: u8 = 15;
const PREDICTED_SPENT: u8 = 20;

#[derive(Clone)]
pub struct FundingRepoRocksDB {
    pub db: Arc<rocksdb::OptimisticTransactionDB>,
}

impl FundingRepoRocksDB {
    pub fn new<P>(db_path: P) -> Self
    where
        P: AsRef<Path>,
    {
        Self {
            db: Arc::new(rocksdb::OptimisticTransactionDB::open_default(db_path).unwrap()),
        }
    }
}

pub struct AvailableFundingBoxes {
    pub confirmed: Vec<FundingBox>,
    pub predicted: Vec<FundingBox>,
}

impl AvailableFundingBoxes {
    pub fn total_lovelaces(&self) -> u64 {
        self.predicted.iter().fold(0, |acc, x| acc + x.value.coin)
            + self.confirmed.iter().fold(0, |acc, x| acc + x.value.coin)
    }
}

#[async_trait::async_trait]
impl FundingRepo for FundingRepoRocksDB {
    async fn collect(&self) -> Result<AvailableFundingBoxes, ()> {
        let db = Arc::clone(&self.db);
        spawn_blocking(move || {
            // Get confirmed boxes first
            let mut confirmed = vec![];
            let prefix = funding_key_prefix(STATE_PREFIX, CONFIRMED_AVAILABLE);
            let mut readopts = ReadOptions::default();
            readopts.set_iterate_range(rocksdb::PrefixRange(prefix.clone()));
            let mut iter = db.iterator_opt(IteratorMode::From(&prefix, Direction::Forward), readopts);
            while let Some(Ok((_key, bytes))) = iter.next() {
                let funding_box: FundingBox = rmp_serde::from_slice(&bytes).unwrap();
                confirmed.push(funding_box);
            }

            // Predicted boxes next
            let mut predicted = vec![];
            let prefix = funding_key_prefix(STATE_PREFIX, PREDICTED_AVAILABLE);
            let mut readopts = ReadOptions::default();
            readopts.set_iterate_range(rocksdb::PrefixRange(prefix.clone()));
            let mut iter = db.iterator_opt(IteratorMode::From(&prefix, Direction::Forward), readopts);
            while let Some(Ok((_key, bytes))) = iter.next() {
                let funding_box: FundingBox = rmp_serde::from_slice(&bytes).unwrap();
                predicted.push(funding_box);
            }
            Ok(AvailableFundingBoxes { confirmed, predicted })
        })
        .await
    }

    async fn put_confirmed(&self, Confirmed(f): Confirmed<FundingBox>) {
        trace!("FB.put_confirmed: {:?}", f);
        let db = self.db.clone();
        let predicted_key = funding_key(STATE_PREFIX, PREDICTED_AVAILABLE, &f.id);
        let confirmed_key = funding_key(STATE_PREFIX, CONFIRMED_AVAILABLE, &f.id);
        spawn_blocking(move || {
            let tx = db.transaction();
            if db.get(&predicted_key).unwrap().is_some() {
                tx.delete(&predicted_key).unwrap();
            }
            tx.put(confirmed_key, rmp_serde::to_vec_named(&f).unwrap())
                .unwrap();
            tx.commit().unwrap();
        })
        .await
    }

    async fn put_predicted(&self, Predicted(f): Predicted<FundingBox>) {
        trace!("FB.put_predicted: {:?}", f);
        let db = self.db.clone();
        let predicted_key = funding_key(STATE_PREFIX, PREDICTED_AVAILABLE, &f.id);
        spawn_blocking(move || {
            db.put(predicted_key, rmp_serde::to_vec_named(&f).unwrap())
                .unwrap();
        })
        .await
    }

    async fn spend_confirmed(&self, f_id: FundingBoxId) {
        let db = self.db.clone();
        let predicted_key = funding_key(STATE_PREFIX, PREDICTED_AVAILABLE, &f_id);
        let confirmed_key = funding_key(STATE_PREFIX, CONFIRMED_AVAILABLE, &f_id);
        spawn_blocking(move || {
            let tx = db.transaction();
            let predicted_spent_key = funding_key(STATE_PREFIX, PREDICTED_SPENT, &f_id);

            if let Some(confirmed_bytes) = db.get(&confirmed_key).unwrap() {
                trace!("FB.spend_confirmed (COMFIRMED AVAILABLE): {:?}", f_id);
                tx.delete(&confirmed_key).unwrap();
                let spent_key = funding_key(STATE_PREFIX, CONFIRMED_SPENT, &f_id);
                tx.put(spent_key, confirmed_bytes).unwrap();
                tx.commit().unwrap();
            } else if let Some(confirmed_bytes) = db.get(&predicted_spent_key).unwrap() {
                trace!("FB.spend_confirmed (PREDICTED SPEND): {:?}", f_id);
                tx.delete(&predicted_spent_key).unwrap();
                let spent_key = funding_key(STATE_PREFIX, CONFIRMED_SPENT, &f_id);
                tx.put(spent_key, confirmed_bytes).unwrap();
                tx.commit().unwrap();
            }
        })
        .await
    }

    async fn unspend_confirmed(&self, f_id: FundingBoxId) {
        let db = self.db.clone();
        let spent_key = funding_key(STATE_PREFIX, CONFIRMED_SPENT, &f_id);
        spawn_blocking(move || {
            if let Some(spent_box_bytes) = db.get(&spent_key).unwrap() {
                trace!("FB.unspend_confirmed: {:?}", f_id);
                let tx = db.transaction();
                tx.delete(&spent_key).unwrap();
                let confirmed_key = funding_key(STATE_PREFIX, CONFIRMED_AVAILABLE, &f_id);
                tx.put(confirmed_key, spent_box_bytes).unwrap();
                tx.commit().unwrap();
            }
        })
        .await
    }

    async fn spend_predicted(&self, f_id: FundingBoxId) {
        trace!("FB.spend_predicted: {:?}", f_id);
        let db = self.db.clone();
        let predicted_key = funding_key(STATE_PREFIX, PREDICTED_AVAILABLE, &f_id);
        spawn_blocking(move || {
            // Can only spend a confirmed UTxO
            let predicted_bytes = db.get(&predicted_key).unwrap().unwrap();

            let tx = db.transaction();
            tx.delete(&predicted_key).unwrap();
            let spent_key = funding_key(STATE_PREFIX, PREDICTED_SPENT, &f_id);
            tx.put(spent_key, predicted_bytes).unwrap();
            tx.commit().unwrap();
        })
        .await
    }

    async fn unspend_predicted(&self, f_id: FundingBoxId) {
        trace!("FB.unspend_predicted: {:?}", f_id);
        let db = self.db.clone();
        let spent_key = funding_key(STATE_PREFIX, PREDICTED_SPENT, &f_id);
        spawn_blocking(move || {
            let spent_box_bytes = db.get(&spent_key).unwrap().unwrap();
            let tx = db.transaction();
            tx.delete(&spent_key).unwrap();

            let predicted_key = funding_key(STATE_PREFIX, PREDICTED_AVAILABLE, &f_id);
            tx.put(predicted_key, spent_box_bytes).unwrap();
            tx.commit().unwrap();
        })
        .await
    }

    async fn eliminate_predicted(&self, f_id: FundingBoxId) {
        trace!("FB.eliminate_predicted: {:?}", f_id);
        let db = self.db.clone();
        let predicted_key = funding_key(STATE_PREFIX, PREDICTED_AVAILABLE, &f_id);
        spawn_blocking(move || {
            db.delete(predicted_key).unwrap();
        })
        .await
    }

    async fn eliminate_confirmed(&self, f_id: FundingBoxId) {
        trace!("FB.eliminate_confirmed: {:?}", f_id);
        let db = self.db.clone();
        let confirmed_key = funding_key(STATE_PREFIX, CONFIRMED_AVAILABLE, &f_id);
        spawn_blocking(move || {
            // Note: we don't assume that the confirmed funding box exists.
            db.delete(confirmed_key).unwrap();
        })
        .await
    }
}

fn funding_key(prefix: &str, seq_num: u8, id: &FundingBoxId) -> Vec<u8> {
    let mut key_bytes = funding_key_prefix(prefix, seq_num);
    let id_bytes = rmp_serde::to_vec(&id).unwrap();
    key_bytes.extend_from_slice(&id_bytes);
    key_bytes
}

fn funding_key_prefix(prefix: &str, seq_num: u8) -> Vec<u8> {
    let mut key_bytes = rmp_serde::to_vec(prefix.as_bytes()).unwrap();
    let seq_num_bytes = rmp_serde::to_vec(&seq_num.to_be_bytes()).unwrap();
    key_bytes.extend_from_slice(&seq_num_bytes);
    key_bytes
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use crate::{
        entities::onchain::funding_box::{FundingBox, FundingBoxId},
        funding::FundingRepo,
    };
    use cml_chain::{assets::MultiAsset, Value};
    use cml_crypto::TransactionHash;
    use rand::{seq::SliceRandom, Rng, RngCore};
    use spectrum_cardano_lib::OutputRef;
    use spectrum_offchain::domain::event::{Confirmed, Predicted};

    use super::FundingRepoRocksDB;

    #[tokio::test]
    async fn test_funding_box() {
        let db = spawn_db();
        let mut funding_boxes: Vec<_> = std::iter::repeat_with(gen_funding_box).take(20).collect();

        for f in &funding_boxes {
            db.put_predicted(Predicted(f.clone())).await;
            db.put_confirmed(Confirmed(f.clone())).await;
        }

        funding_boxes.sort_by(|f0, f1| f0.id.cmp(&f1.id));
        let mut collected = db.collect().await.unwrap();
        assert!(collected.predicted.is_empty());
        collected.confirmed.sort_by(|f0, f1| f0.id.cmp(&f1.id));
        assert_eq!(collected.confirmed, funding_boxes);

        // Add 50 predicted funding boxes, which will have no impact on collection.
        for _ in 0..50 {
            db.put_predicted(Predicted(gen_funding_box())).await;
        }

        // Randomly remove 5 confirmed funding boxes
        let mut rng = rand::thread_rng();
        funding_boxes.shuffle(&mut rng);
        for _ in 0..5 {
            let f = funding_boxes.pop().unwrap();
            db.spend_predicted(f.id).await;
            db.spend_confirmed(f.id).await;
        }

        funding_boxes.sort_by(|f0, f1| f0.id.cmp(&f1.id));
        let mut available = db.collect().await.unwrap();
        available.predicted.extend(available.confirmed);
        let mut collected = available.predicted;
        collected.sort_by(|f0, f1| f0.id.cmp(&f1.id));
        assert_eq!(collected, funding_boxes);
    }

    #[tokio::test]
    async fn test_unspend_predicted() {
        let db = spawn_db();
        let fb = vec![gen_funding_box()];
        db.put_predicted(Predicted(fb[0].clone())).await;
        db.spend_predicted(fb[0].id).await;
        db.unspend_predicted(fb[0].id).await;
        assert_eq!(fb, db.collect().await.unwrap().predicted);
    }

    #[tokio::test]
    async fn test_unspend_confirmed() {
        let db = spawn_db();
        let fb = vec![gen_funding_box()];
        db.put_confirmed(Confirmed(fb[0].clone())).await;
        db.spend_confirmed(fb[0].id).await;
        db.unspend_confirmed(fb[0].id).await;
        assert_eq!(fb, db.collect().await.unwrap().confirmed);
    }

    fn spawn_db() -> FundingRepoRocksDB {
        let rnd = rand::thread_rng().next_u32();
        let db = Arc::new(rocksdb::OptimisticTransactionDB::open_default(format!("./tmp/{}", rnd)).unwrap());
        FundingRepoRocksDB { db }
    }

    fn gen_funding_box() -> FundingBox {
        let mut rng = rand::thread_rng();
        let mut random_bytes: [u8; 32] = [0; 32];
        rng.fill(&mut random_bytes);
        let id = FundingBoxId::from(OutputRef::new(TransactionHash::from(random_bytes), 0));
        let value = Value::new(100000, MultiAsset::new());
        FundingBox { id, value }
    }
}
