use crate::account::AccountInPool;
use cml_chain::certs::Credential;
use serde::{Deserialize, Serialize};
use spectrum_offchain_cardano::data::PoolId;

#[derive(Debug, Serialize, Deserialize)]
pub struct ExportAccountEvent {
    pub account_cred: Credential,
    pub pool_id: PoolId,
    pub update: AccountInPool,
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use crate::account::AccountInPool;
    use crate::feed::event::ExportAccountEvent;
    use cml_chain::certs::Credential;
    use cml_chain::PolicyId;
    use cml_crypto::Ed25519KeyHash;
    use rocksdb::{ColumnFamily, IteratorMode, Options, ReadOptions, TransactionDB, TransactionDBOptions};
    use tokio::task::spawn_blocking;
    use spectrum_cardano_lib::{AssetName, Token};
    use spectrum_offchain_cardano::data::PoolId;
    use crate::position_db::{pool_key, AGGREGATE_CF, COLUMN_FAMILIES, EVENTS_CF, POOL_LQ_FRAMES_INDEX_CF};

    #[tokio::test]
    async fn json_sample() {

        let mut opts = Options::default();
        opts.create_if_missing(true);
        opts.create_missing_column_families(true);
        opts.set_db_log_dir("/Users/aleksandr/IdeaProjects/spectrum-offchain-multiplatform/splash-lp-indexer/rocksdb-logs");
        let db_opts = TransactionDBOptions::default();
        let db: Arc<TransactionDB> = Arc::new(TransactionDB::open_cf(&opts, &db_opts, "/Users/aleksandr/IdeaProjects/spectrum-offchain-multiplatform/splash-lp-indexer/index/accounts", COLUMN_FAMILIES).unwrap());

        let db_1 = db.clone();
        let commit_test = spawn_blocking(move || {
            let pool_id_1 = PoolId(
                Token(
                    PolicyId::from_hex("b5e8836450525a945852ad7e1648dc8ce26bfc4d495496bfdca834e4").unwrap(),
                    AssetName::from_utf8("ddd_ADA_NFT".to_string())
                )
            );

            let pool_id_2 = PoolId(
                Token(
                    PolicyId::from_hex("409693984e23d579fd67f1fe78eb9d695ecbd75afdd73dc3405e55f8").unwrap(),
                    AssetName::from_utf8("qwe_ADA_NFT".to_string())
                )
            );

            let pool_id_3 = PoolId(
                Token(
                    PolicyId::from_hex("559627248e6aa2c7f9e20e6d6536d94251bc2e9fff1a66ce4125bdf4").unwrap(),
                    AssetName::from_utf8("testD_ADA_NFT".to_string())
                )
            );

            let pool_1_key = pool_key(pool_id_1);
            let pool_2_key = pool_key(pool_id_2);
            let pool_3_key = pool_key(pool_id_3);

            let cf_handle = db_1.cf_handle(POOL_LQ_FRAMES_INDEX_CF).expect("CF must exist");

            let tx = db_1.transaction();

            tx.put_cf(
                cf_handle,
                pool_1_key.clone(),
                &rmp_serde::to_vec(&123).unwrap()
            ).unwrap();

            tx.put_cf(
                cf_handle,
                pool_2_key.clone(),
                &rmp_serde::to_vec(&123).unwrap()
            ).unwrap();

            tx.put_cf(
                cf_handle,
                pool_3_key.clone(),
                &rmp_serde::to_vec(&123).unwrap()
            ).unwrap();

            tx.commit().unwrap();

            let pool_id_1 = PoolId(
                Token(
                    PolicyId::from_hex("b5e8836450525a945852ad7e1648dc8ce26bfc4d495496bfdca834e4").unwrap(),
                    AssetName::from_utf8("ddd_ADA_NFT".to_string())
                )
            );

            let pool_id_2 = PoolId(
                Token(
                    PolicyId::from_hex("409693984e23d579fd67f1fe78eb9d695ecbd75afdd73dc3405e55f8").unwrap(),
                    AssetName::from_utf8("qwe_ADA_NFT".to_string())
                )
            );

            let pool_id_3 = PoolId(
                Token(
                    PolicyId::from_hex("559627248e6aa2c7f9e20e6d6536d94251bc2e9fff1a66ce4125bdf4").unwrap(),
                    AssetName::from_utf8("testD_ADA_NFT".to_string())
                )
            );

            let pool_1_key = pool_key(pool_id_1);
            let pool_2_key = pool_key(pool_id_2);
            let pool_3_key = pool_key(pool_id_3);

            let new_tx = db_1.transaction();
            let test = db_1.cf_handle(POOL_LQ_FRAMES_INDEX_CF).expect("CF must exist");

            let result_1 = new_tx.get_cf(test, pool_1_key.clone()).unwrap();
            let result_2 = new_tx.get_cf(test, pool_2_key.clone()).unwrap();
            let result_3 = new_tx.get_cf(test, pool_3_key.clone()).unwrap();

            println!("{:?} {:?} {:?}", result_1.is_some(), result_2.is_some(), result_3.is_some());

            new_tx.commit().unwrap();
        }).await.unwrap();

        let sample = ExportAccountEvent {
            account_cred: Credential::new_pub_key(Ed25519KeyHash::from([0u8; 28])),
            pool_id: PoolId::random(),
            update: AccountInPool::new(1, true),
        };
        println!("{}", serde_json::to_string(&sample).unwrap());
    }
}
