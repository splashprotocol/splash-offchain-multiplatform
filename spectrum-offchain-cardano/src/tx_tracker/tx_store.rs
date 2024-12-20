use rocksdb::{Direction, IteratorMode, ReadOptions};
use std::marker::PhantomData;
use std::sync::Arc;

pub(crate) struct TxStore<TxHash, Tx> {
    db: Arc<rocksdb::OptimisticTransactionDB>,
    pd: PhantomData<(TxHash, Tx)>,
}

impl<TxHash, Tx> TxStore<TxHash, Tx> {
    pub fn new(db: Arc<rocksdb::OptimisticTransactionDB>) -> Self {
        Self { db, pd: PhantomData }
    }

    pub(crate) async fn insert(&self, block_num: u64, tx_hash: TxHash, tx: Tx)
    where
        TxHash: Send + cml_core::serialization::RawBytesEncoding + 'static,
        Tx: Send + cml_core::serialization::Serialize + 'static,
    {
        let db = self.db.clone();
        tokio::task::spawn_blocking(move || {
            db.put(full_key(block_num, tx_hash), tx.to_cbor_bytes()).unwrap();
        })
        .await
        .unwrap();
    }

    pub(crate) async fn remove(&self, block_num: u64, tx_hash: TxHash)
    where
        TxHash: Send + cml_core::serialization::RawBytesEncoding + 'static,
    {
        let db = self.db.clone();
        tokio::task::spawn_blocking(move || {
            db.delete(full_key(block_num, tx_hash)).unwrap();
        })
        .await
        .unwrap();
    }

    pub(crate) async fn remove_all(&self, block_num: u64) -> Vec<(TxHash, Tx)>
    where
        TxHash: Send + cml_core::serialization::RawBytesEncoding + 'static,
        Tx: Send + cml_core::serialization::Deserialize + 'static,
    {
        let db = Arc::clone(&self.db);
        tokio::task::spawn_blocking(move || {
            let prefix = block_key(block_num);
            let mut readopts = ReadOptions::default();
            readopts.set_iterate_range(rocksdb::PrefixRange(prefix.clone()));
            let mut iter = db.iterator_opt(IteratorMode::From(&prefix, Direction::Forward), readopts);
            let mut bf = vec![];
            while let Some(Ok((key, bytes))) = iter.next() {
                let tx_hash = from_full_key(key.clone().to_vec()).unwrap();
                let tx = Tx::from_cbor_bytes(bytes.as_ref()).unwrap();
                db.delete(key).unwrap();
                bf.push((tx_hash, tx));
            }
            bf
        })
        .await
        .unwrap()
    }
}

fn full_key<TxHash: cml_core::serialization::RawBytesEncoding>(block_num: u64, tx_hash: TxHash) -> Vec<u8> {
    let mut bf = vec![];
    bf.extend_from_slice(block_num.to_be_bytes().as_ref());
    bf.extend_from_slice(tx_hash.to_raw_bytes());
    bf
}

fn from_full_key<TxHash: cml_core::serialization::RawBytesEncoding>(key: Vec<u8>) -> Option<TxHash> {
    TxHash::from_raw_bytes(&key[8..]).ok()
}

fn block_key(block_num: u64) -> Vec<u8> {
    block_num.to_be_bytes().to_vec()
}

#[cfg(test)]
mod tests {
    use crate::tx_tracker::tx_store::TxStore;
    use cml_chain::assets::AssetName;
    use cml_chain::crypto::TransactionHash;
    use rocksdb::{OptimisticTransactionDB, Options, SingleThreaded};
    use std::sync::Arc;

    #[tokio::test]
    async fn insert_remove_remove_remaining() {
        let path = "_path_for_rocksdb_storage2";
        {
            let db = OptimisticTransactionDB::open_default(path).unwrap();
            let store: TxStore<TransactionHash, AssetName> = TxStore {
                db: Arc::new(db),
                pd: Default::default(),
            };
            let tx_hash_1 =
                TransactionHash::from_hex("660612f02f8bea29c3872b84b7b9510b5cc31931f81de747a8d072e0a45ca3ec")
                    .unwrap();
            let tx_hash_2 =
                TransactionHash::from_hex("0db5541c95c1fb55f31dd15fb370b331d61a197f4ccb6d1cc5fa198440fed0fa")
                    .unwrap();
            let tx_hash_3 =
                TransactionHash::from_hex("e8adbd0e75d3e2be9187e30857ba2b8f215e2c8389cf9dc55b5858633ba62612")
                    .unwrap();
            let tx_hash_4 =
                TransactionHash::from_hex("80d0dc4d2c66d651163dd63401a8c9bc95286bc395cd40c43ba17d397781c7a9")
                    .unwrap();
            let tx_hash_5 =
                TransactionHash::from_hex("f462ae69a973229e40e3c3ac8e1b75c74be27dfeccbe6ce9fb073375af77623a")
                    .unwrap();
            let tx_1 = AssetName::new(vec![1]).unwrap();
            let tx_2 = AssetName::new(vec![2]).unwrap();
            let tx_3 = AssetName::new(vec![3]).unwrap();
            let tx_4 = AssetName::new(vec![4]).unwrap();
            let tx_5 = AssetName::new(vec![5]).unwrap();
            store.insert(1, tx_hash_1, tx_1).await;
            store.insert(1, tx_hash_2, tx_2.clone()).await;
            store.insert(1, tx_hash_3, tx_3.clone()).await;
            store.insert(5, tx_hash_4, tx_4.clone()).await;
            store.insert(5, tx_hash_5, tx_5.clone()).await;

            store.remove(1, tx_hash_1).await;
            let remaining = store.remove_all(1).await;
            assert_eq!(remaining, vec![(tx_hash_2, tx_2), (tx_hash_3, tx_3)]);
            let remaining = store.remove_all(5).await;
            assert_eq!(remaining, vec![(tx_hash_4, tx_4), (tx_hash_5, tx_5)]);
        }
        let _ = OptimisticTransactionDB::<SingleThreaded>::destroy(&Options::default(), path);
    }
}
