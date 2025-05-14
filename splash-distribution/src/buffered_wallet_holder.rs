use crate::entities::buffered_wallet::{BufferedWallet, BufferedWalletStatus};
use bloom_offchain::execution_engine::bundled::Bundled;
use cml_core::serialization::RawBytesEncoding;
use log::info;
use rocksdb::{Direction, IteratorMode, Options, ReadOptions};
use spectrum_cardano_lib::output::FinalizedTxOut;
use spectrum_cardano_lib::{AssetClass, Token};
use spectrum_offchain_cardano::data::pair::PairId;
use spectrum_offchain_cardano::data::pool::ImmutablePoolUtxo;
use splash_dao_offchain::entities::onchain::funding_box::FundingBox;
use splash_dao_offchain::funding::AvailableFundingBoxes;
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use tokio::task::spawn_blocking;

pub struct BufferedWalletsHolder {
    pub db: Arc<rocksdb::OptimisticTransactionDB>,
}

const TABLES: [&str; 1] = ["new"];

impl BufferedWalletsHolder {
    fn buffered_wallet_key(buffered_wallet: Bundled<BufferedWallet, FinalizedTxOut>) -> Vec<u8> {
        let mut key = buffered_wallet.0.id.tx_hash().to_raw_bytes().to_vec();
        key.extend(rmp_serde::to_vec(&buffered_wallet.0.id.index()).unwrap());
        key
    }

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

    pub async fn get_wallets_and_reserve_for_withdraw(
        &mut self,
        pair: Token,
        splash_amount_to_withdraw: u64,
        lovelace_amount_to_withdraw: u64,
    ) -> Vec<Bundled<BufferedWallet, FinalizedTxOut>> {
        info!("get_wallets_and_reserved_for_withdraw");

        let db = Arc::clone(&self.db);

        spawn_blocking(move || {
            let mut accumulated_splash_qty = 0;
            let mut splash_wallet_to_reserve: Vec<Bundled<BufferedWallet, FinalizedTxOut>> = vec![];
            // todo: is not used in first version. Only splash can be distributed
            //let mut lovelace_wallet_to_reserve = vec![];
            let wallets_cf = db.cf_handle(TABLES[0]).unwrap();
            let tx = db.transaction();
            let wallets_to_return = {
                let mut to_return = vec![];
                let mut iter_events =
                    tx.iterator_cf_opt(wallets_cf, ReadOptions::default(), IteratorMode::Start);
                while let Some(Ok((_, raw_wallet_info))) = iter_events.next() {
                    if accumulated_splash_qty >= splash_amount_to_withdraw {
                        splash_wallet_to_reserve.into_iter().for_each(|bundled| {
                            let mut updated_buffered_wallet: BufferedWallet = bundled.0.clone();
                            updated_buffered_wallet.status = BufferedWalletStatus::UserWithdraw;
                            tx.put_cf(
                                wallets_cf,
                                Self::buffered_wallet_key(bundled.clone()),
                                rmp_serde::to_vec(&Bundled(updated_buffered_wallet, bundled.1.clone()))
                                    .unwrap(),
                            )
                            .unwrap();

                            to_return.push(Bundled(updated_buffered_wallet, bundled.1));
                        });

                        return to_return;
                    }
                    let parsed_wallet_info: Bundled<BufferedWallet, FinalizedTxOut> =
                        rmp_serde::from_slice(&raw_wallet_info).unwrap();
                    accumulated_splash_qty += parsed_wallet_info.0.splash_amount;
                    splash_wallet_to_reserve.push(parsed_wallet_info);
                }

                to_return
            };

            tx.commit().unwrap();

            wallets_to_return
        })
        .await
        .unwrap()
    }

    pub async fn free_buffered_wallets(&mut self) -> () {
        info!("free_buffered_wallets");
        // spawn_blocking(move || {
        //
        // }).await
        unimplemented!()
    }

    pub async fn get_wallet_to_smart_farm_withdraw(&mut self) -> BufferedWallet {
        info!("get_wallet_to_smart_farm_withdraw");
        unimplemented!()
    }
}
