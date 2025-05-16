use async_trait::async_trait;
use cml_chain::certs::Credential;
use cml_chain::transaction::TransactionOutput;
use cml_chain::{Deserialize, Serialize};
use cml_crypto::{Ed25519KeyHash, RawBytesEncoding, TransactionHash};
use rocksdb::{
    ColumnFamily, DBIteratorWithThreadMode, Direction, IteratorMode, Options, ReadOptions,
    SnapshotWithThreadMode, TransactionDB, TransactionDBOptions,
};
use spectrum_cardano_lib::OutputRef;
use spectrum_offchain::domain::event::Confirmed;
use std::path::Path;
use std::sync::Arc;
use tokio::task::spawn_blocking;

#[async_trait]
pub trait UtxoIndex {
    async fn apply(
        &self,
        tx_hash: TransactionHash,
        inputs: Vec<OutputRef>,
        outputs: Vec<(usize, TransactionOutput)>,
        confirmed: bool,
    );
    async fn unapply(
        &self,
        tx_hash: TransactionHash,
        inputs: Vec<OutputRef>,
        outputs: Vec<(usize, TransactionOutput)>,
    );
}

#[async_trait]
pub trait UtxoResolver {
    async fn get_utxos(
        &self,
        pkh: Ed25519KeyHash,
        offset: usize,
        limit: usize,
    ) -> Vec<(OutputRef, (TransactionOutput, bool))>;
}

#[derive(Clone)]
pub struct RocksDB {
    db: Arc<TransactionDB>,
}

impl RocksDB {
    pub fn new<P: AsRef<Path>>(path: P) -> Self {
        let mut opts = Options::default();
        opts.create_if_missing(true);
        opts.create_missing_column_families(true);
        let db_opts = TransactionDBOptions::default();
        Self {
            db: Arc::new(TransactionDB::open_cf(&opts, &db_opts, path, TABLES).unwrap()),
        }
    }
}

const TABLES: [&str; 3] = ["utxos", "spent_utxos", "addrs"];

fn utxo_key(rf: OutputRef) -> Vec<u8> {
    rmp_serde::to_vec(&rf).unwrap()
}

fn pkh_key(pkh: &Ed25519KeyHash) -> Vec<u8> {
    pkh.to_raw_bytes().to_vec()
}

fn pkh_to_utxo_key(pkh: &Ed25519KeyHash, rf: OutputRef) -> Vec<u8> {
    let mut bf = pkh_key(pkh);
    bf.extend(utxo_key(rf));
    bf
}

fn unsafe_utxo_key_from_index(pkh_len: usize, bytes: &[u8]) -> &[u8] {
    &bytes[pkh_len..]
}

#[async_trait]
impl UtxoIndex for RocksDB {
    async fn apply(
        &self,
        tx_hash: TransactionHash,
        inputs: Vec<OutputRef>,
        outputs: Vec<(usize, TransactionOutput)>,
        confirmed: bool,
    ) {
        let db = self.db.clone();
        spawn_blocking(move || {
            let utxos = db.cf_handle(TABLES[0]).unwrap();
            let spent_utxos = db.cf_handle(TABLES[1]).unwrap();
            let addrs = db.cf_handle(TABLES[2]).unwrap();
            let tx = db.transaction();
            for i in inputs {
                if let Some(out) = tx
                    .get_cf(utxos, utxo_key(i))
                    .unwrap()
                    .and_then(|bytes| TransactionOutput::from_cbor_bytes(&bytes).ok())
                {
                    if let Some(Credential::PubKey { hash, .. }) = out.address().payment_cred() {
                        tx.put_cf(spent_utxos, utxo_key(i), vec![]).unwrap();
                        tx.delete_cf(addrs, pkh_to_utxo_key(hash, i)).unwrap();
                    }
                }
            }
            for (i, o) in outputs {
                if let Some(Credential::PubKey { hash, .. }) = o.address().payment_cred() {
                    let rf = OutputRef::new(tx_hash, i as u64);
                    let bytes = rmp_serde::to_vec(&(o.to_canonical_cbor_bytes(), confirmed)).unwrap();
                    tx.put_cf(utxos, utxo_key(rf), bytes).unwrap();
                    tx.put_cf(addrs, pkh_to_utxo_key(hash, rf), vec![]).unwrap();
                }
            }
            tx.commit().unwrap();
        })
        .await
        .unwrap()
    }

    async fn unapply(
        &self,
        tx_hash: TransactionHash,
        inputs: Vec<OutputRef>,
        outputs: Vec<(usize, TransactionOutput)>,
    ) {
        let db = self.db.clone();
        spawn_blocking(move || {
            let utxos = db.cf_handle(TABLES[0]).unwrap();
            let spent_utxos = db.cf_handle(TABLES[1]).unwrap();
            let addrs = db.cf_handle(TABLES[2]).unwrap();
            let tx = db.transaction();
            for rf in inputs {
                if let Some(out) = tx
                    .get_cf(utxos, utxo_key(rf))
                    .unwrap()
                    .and_then(|bytes| TransactionOutput::from_cbor_bytes(&bytes).ok())
                {
                    if let Some(Credential::PubKey { hash, .. }) = out.address().payment_cred() {
                        tx.delete_cf(spent_utxos, utxo_key(rf)).unwrap();
                        tx.put_cf(addrs, pkh_to_utxo_key(hash, rf), vec![]).unwrap();
                    }
                }
            }
            for (ix, o) in outputs {
                if let Some(Credential::PubKey { hash, .. }) = o.address().payment_cred() {
                    let rf = OutputRef::new(tx_hash, ix as u64);
                    tx.delete_cf(utxos, utxo_key(rf)).unwrap();
                    tx.delete_cf(addrs, pkh_to_utxo_key(hash, rf)).unwrap();
                }
            }
            tx.commit().unwrap();
        })
        .await
        .unwrap()
    }
}

#[async_trait]
impl UtxoResolver for RocksDB {
    async fn get_utxos(
        &self,
        pkh: Ed25519KeyHash,
        offset: usize,
        limit: usize,
    ) -> Vec<(OutputRef, (TransactionOutput, bool))> {
        let db = self.db.clone();
        spawn_blocking(move || {
            let utxos = db.cf_handle(TABLES[0]).unwrap();
            let spent_utxos = db.cf_handle(TABLES[1]).unwrap();
            let addrs = db.cf_handle(TABLES[2]).unwrap();
            let snap = db.snapshot();
            let pkh_key = pkh_key(&pkh);
            let pkh_key_len = pkh_key.len();
            let mut utxos_at_address = get_range_iterator(&snap, addrs, pkh_key).skip(offset).take(limit);
            let mut utxo_set = vec![];
            while let Some(Ok(bytes)) = utxos_at_address.next() {
                let utxo_key = unsafe_utxo_key_from_index(pkh_key_len, &bytes.0);
                let unspent = snap.get_cf(spent_utxos, utxo_key).unwrap().is_none();
                if unspent {
                    if let Some(utxo) = snap
                        .get_cf(utxos, utxo_key)
                        .unwrap()
                        .and_then(|bytes| rmp_serde::from_slice::<(Vec<u8>, bool)>(&bytes).ok())
                        .and_then(|(utxo_bytes, confirmed)| {
                            TransactionOutput::from_cbor_bytes(&utxo_bytes)
                                .ok()
                                .map(|o| (o, confirmed))
                        })
                    {
                        let fr = rmp_serde::from_slice::<OutputRef>(&utxo_key).unwrap();
                        utxo_set.push((fr, utxo));
                    }
                }
            }
            utxo_set
        })
        .await
        .unwrap()
    }
}

pub(crate) fn get_range_iterator<'a: 'b, 'b>(
    db: &'a SnapshotWithThreadMode<'b, TransactionDB>,
    cf: &ColumnFamily,
    prefix: Vec<u8>,
) -> DBIteratorWithThreadMode<'b, TransactionDB> {
    let mut readopts = ReadOptions::default();
    readopts.set_iterate_range(rocksdb::PrefixRange(prefix.clone()));
    db.iterator_cf_opt(cf, readopts, IteratorMode::From(&prefix, Direction::Forward))
}

#[cfg(test)]
mod tests {
    use crate::index::{RocksDB, UtxoIndex, UtxoResolver};
    use cml_chain::address::Address;
    use cml_chain::certs::Credential;
    use cml_chain::transaction::Transaction;
    use cml_chain::Deserialize;
    use rocksdb::{Options, SingleThreaded, TransactionDB};
    use spectrum_offchain::tx_hash::CanonicalHash;
    use std::path::{Path, PathBuf};

    #[tokio::test]
    async fn index_applied_transactions() {
        let db_path = DBPath::new("_index_applied_transactions");
        let db = RocksDB::new(&db_path);

        let addr = Address::from_bech32(ADDR).unwrap();
        let Credential::PubKey { hash: pkh, .. } = addr.payment_cred().unwrap() else {
            panic!()
        };

        for rtx in [TX_FUN_1, TX_FUN_2, TX_ORD, TX_EXE] {
            let tx = Transaction::from_cbor_bytes(&*hex::decode(rtx).unwrap()).unwrap();
            let hash = tx.canonical_hash();
            db.apply(
                hash,
                tx.body.inputs.into_iter().map(|i| i.into()).collect(),
                tx.body.outputs.to_vec().into_iter().enumerate().collect(),
                true,
            )
            .await;
        }

        let utxos = db.get_utxos(*pkh, 0, 100).await;

        dbg!(&utxos);
    }

    const ADDR: &str = "addr1qxm9vre80nsqjtsp7w0u756t9ea9s2pzvr8sg3f878nlwnfnk2t57etkfqvkjup3udn836gra978y0pkf2selr94zqlqy7hy0z";

    const TX_FUN_1: &str = "84a300838258206941cb48d7cb81680dc819afaa08c8822542a06e0adb28886cc100a33eb6aa9301825820b54aa6b7fa267f7c21cc053b3ecc629ba8ec32e320c65a85d3de327571bad14a06825820fefd3112b84fe034d35c023ccdb480e15a637efd7be2c2012fcc281363eae336010182a300583911464eeee89f05aff787d40045af2a40a83fd96c513197d32fbc54ff0233b2974f65764819697031e36678e903e97c723c364aa19f8cb5103e011a0073f780028201d81858f5d8798c4100581c07f03034afc822b3f2921e504a21e130fddbeafffa9a215660f87289d8798240401a004c4b401a000927c0194bf7d87982581cabb15dbbcc5c7c80cebea450f4f2131ec1f5b27ca38b66418e4c9a5144424f4241d879821a003b593a1a3b9aca001a0007a120d87982d87981581cb6560f277ce0092e01f39fcf534b2e7a58282260cf044527f1e7f74dd87981d87981d87981581c33b2974f65764819697031e36678e903e97c723c364aa19f8cb5103e581cb6560f277ce0092e01f39fcf534b2e7a58282260cf044527f1e7f74d81581c9beb201348b07d30ee9370b0c353fb0ef566a4c79b153477f15ccf9482583901b6560f277ce0092e01f39fcf534b2e7a58282260cf044527f1e7f74d33b2974f65764819697031e36678e903e97c723c364aa19f8cb5103e1a002d7991021a0002c87da10081825820b1b1f9da358b4e88657bd65bcf3d69bf3f21d37ce4a258e57e0637722510ea5b58408a989cd1e1f45de9a66ad43fe6dc7e22122a7e2063743aab977e128f7263649212015205fdac2b6ffc337b38fecfe4b61d7514bedf6791ec67ae1b1cf58e7b0cf5f6";
    const TX_FUN_2: &str = "84a8008d825820020240e6044c37a067e28dcee32f5caec36c0f9946731731d59de5caebf8359b0082582025e595079ac63aa31e971568690f262880d2b83973a1584a78a88fb01dd5437b0082582043ec53f8b883883e1c81a5be3cf902889c7b5e75c97e86fc5e260a8e095b160400825820440d0bb54c9a6966393e838421251524a5c30a6611bce0a5c538d91f41bfda30008258204a1901e06c5f6686e8ef24f1ecfdfb6fa62c97ff5998c63e78eb16a9a0fb4bc800825820515b9da914e3b9b38e21b9c32fd914ae0111c29ae8fd9ce595afa1c7d7edc0c3018258207b201250e6706a9657f8833a926f5dcaf95c6b0b42a324bfd466f4f369892bcc00825820878bc22cd9a223d0d566d459d52acc8728e8e35dc9b3c293be148de0715c2b19008258208b437749397385ffd0f2413e2945433f15d6221202812294aefe1d0f761fcacf00825820a58f42d16ea1d31d14e527b0c389100e711c1041abee65529c8caa8f16ed687900825820cc6e5aae7a229244775bb6004f0468a48fe4e5521aa6dfe80fab328140654a6900825820d66747d9e69a4d61d9522eb4872544ad728d4049114b9a47a35342606afa107700825820fefd3112b84fe034d35c023ccdb480e15a637efd7be2c2012fcc281363eae33600018ea200583901719bee424a97b58b3dca88fe5da6feac6494aa7226f975f3506c5b257846f6bb07f5b2825885e4502679e699b4e60a0c4609a46bc35454cd01821a0016e360a1581cabb15dbbcc5c7c80cebea450f4f2131ec1f5b27ca38b66418e4c9a51a144424f42411a0011b71aa2005839015eea0414fdf74d68a82e9d74e0c0e73f823758a533fc67b2de1f590a6918226667b4c67a551249bcf13b6a87488f14f9740bb30bee37185101821a0016e360a1581cabb15dbbcc5c7c80cebea450f4f2131ec1f5b27ca38b66418e4c9a51a144424f42411a001d9c14a20058390155aa458e1288691f5467638dc215385423a27ba6cddaf44240dc159f8c639260161c1aa71f77b79ec56f80643b9823408423ba3ef4f73aae01821a0016e360a1581cabb15dbbcc5c7c80cebea450f4f2131ec1f5b27ca38b66418e4c9a51a144424f42411a0005e3dfa200583901b6560f277ce0092e01f39fcf534b2e7a58282260cf044527f1e7f74d33b2974f65764819697031e36678e903e97c723c364aa19f8cb5103e01821a0016e360a1581cabb15dbbcc5c7c80cebea450f4f2131ec1f5b27ca38b66418e4c9a51a144424f42411a001daca1a2005839015eea0414fdf74d68a82e9d74e0c0e73f823758a533fc67b2de1f590a6918226667b4c67a551249bcf13b6a87488f14f9740bb30bee37185101821a0016e360a1581cabb15dbbcc5c7c80cebea450f4f2131ec1f5b27ca38b66418e4c9a51a144424f42411a0005e53ba300583931905ab869961b094f1b8197278cfe15b45cbe49fa8f32c6b014f85a2db2f6abf60ccde92eae1a2f4fdf65f2eaf6208d872c6f0e597cc10b0701821a022f6320a2581c63f947b8d9535bc4e4ce6919e3dc056547e8d30ada12f29aa5f826b8a15820588e3c07f5c2119882784077cb720649309bb0a024383d44f5436c9c5c15cbfe01581cabb15dbbcc5c7c80cebea450f4f2131ec1f5b27ca38b66418e4c9a51a144424f42411a3ad18dad028201d81858e2d87989d87982581c63f947b8d9535bc4e4ce6919e3dc056547e8d30ada12f29aa5f826b85820588e3c07f5c2119882784077cb720649309bb0a024383d44f5436c9c5c15cbfed879824040d87982581cabb15dbbcc5c7c80cebea450f4f2131ec1f5b27ca38b66418e4c9a5144424f42411b0000001c871b063f1a0026d61e581c9beb201348b07d30ee9370b0c353fb0ef566a4c79b153477f15ccf941b000000043c4abc40581c8807fbe6e36b1c35ad6f36f0993e2fc67ab6f2db06041cfa3a53c04a581c30c1003aa7dec834e0d0a78db547ba8840e58060725dbfae352f0d64a20058390155aa458e1288691f5467638dc215385423a27ba6cddaf44240dc159f8c639260161c1aa71f77b79ec56f80643b9823408423ba3ef4f73aae01821a0016e360a1581cabb15dbbcc5c7c80cebea450f4f2131ec1f5b27ca38b66418e4c9a51a144424f42411a0005e490a2005839015eea0414fdf74d68a82e9d74e0c0e73f823758a533fc67b2de1f590a6918226667b4c67a551249bcf13b6a87488f14f9740bb30bee37185101821a0016e360a1581cabb15dbbcc5c7c80cebea450f4f2131ec1f5b27ca38b66418e4c9a51a144424f42411a001da454a200583901719bee424a97b58b3dca88fe5da6feac6494aa7226f975f3506c5b257846f6bb07f5b2825885e4502679e699b4e60a0c4609a46bc35454cd01821a0016e360a1581cabb15dbbcc5c7c80cebea450f4f2131ec1f5b27ca38b66418e4c9a51a144424f42411a0005e683a20058390155aa458e1288691f5467638dc215385423a27ba6cddaf44240dc159f8c639260161c1aa71f77b79ec56f80643b9823408423ba3ef4f73aae01821a0016e360a1581cabb15dbbcc5c7c80cebea450f4f2131ec1f5b27ca38b66418e4c9a51a144424f42411a0005e5e2a200583901566e753a9c91b020b32d333eab77c694c251d254cc6025eff3927e26fffc46e7476fe35be30c9061af4c718225a1a60274a9c22e048af00401821a0016e360a1581cabb15dbbcc5c7c80cebea450f4f2131ec1f5b27ca38b66418e4c9a51a144424f42411a001d911fa2005839015eea0414fdf74d68a82e9d74e0c0e73f823758a533fc67b2de1f590a6918226667b4c67a551249bcf13b6a87488f14f9740bb30bee37185101821a0016e360a1581cabb15dbbcc5c7c80cebea450f4f2131ec1f5b27ca38b66418e4c9a51a144424f42411a0005e329a200583901b6560f277ce0092e01f39fcf534b2e7a58282260cf044527f1e7f74d33b2974f65764819697031e36678e903e97c723c364aa19f8cb5103e01821a0016e360a1581cabb15dbbcc5c7c80cebea450f4f2131ec1f5b27ca38b66418e4c9a51a144424f42411a001da9d98258390122aea2da15e494e01767145d48bda16b6d437f1c449823a044193daf299a82ef56311aa10adf04c0072d4870eb9f4d5ff315132434841b741a00c09029021a000e0a7705a1581df196f5c1bee23481335ff4aece32fe1dfa1aa40a944a66d2d6edc9a9a5000b58200bb1f94e938617dc9fdb251101aea6fafd37165e30641d92924b39b58cb191610d81825820b54aa6b7fa267f7c21cc053b3ecc629ba8ec32e320c65a85d3de327571bad14a040e81581c9beb201348b07d30ee9370b0c353fb0ef566a4c79b153477f15ccf941283825820c4a540ac2e06c217dd4fb3f39ca3863da394ba134677dafa9b98830ca71d584d03825820b91eda29d145ab6c0bc0d6b7093cb24b131440b7b015033205476f39c690a51f00825820b91eda29d145ab6c0bc0d6b7093cb24b131440b7b015033205476f39c690a51f01a200818258208f11dc37d81c0dff768d41bbb1bbc30328283183fd608bcb2eec9ccbafc1c52a5840ae87010d2c0fbf0effecfcd14fb2bf34f6c8214abcb79d676c74a6ececd8fc224a5903f472fd636b1bc2aaf5031abeb1728edf7b50bb2ab74c754eb01c907807058e840000d87a80821a000186a01a01c9c380840001d87a80821a000186a01a01c9c380840002d87a80821a000186a01a01c9c380840003d87a80821a000186a01a01c9c380840004d87a80821a000186a01a01c9c380840005d879830505d87980821a000864701a0d1cef00840006d87a80821a000186a01a01c9c380840007d87a80821a000186a01a01c9c380840008d87a80821a000186a01a01c9c380840009d87a80821a000186a01a01c9c38084000ad87a80821a000186a01a01c9c38084000bd87a80821a000186a01a01c9c38084000cd87a80821a000186a01a01c9c38084030080821a0029a8101a3dfd2400f5f6";
    const TX_ORD: &str = "84a30083825820440d0bb54c9a6966393e838421251524a5c30a6611bce0a5c538d91f41bfda3001825820b550f1a6365cae7b3464b7c57352915c7cc46043666aa40378f3c425da706bd703825820b550f1a6365cae7b3464b7c57352915c7cc46043666aa40378f3c425da706bd70c0182a300583911464eeee89f05aff787d40045af2a40a83fd96c513197d32fbc54ff0233b2974f65764819697031e36678e903e97c723c364aa19f8cb5103e01821a0027ac40a1581cabb15dbbcc5c7c80cebea450f4f2131ec1f5b27ca38b66418e4c9a51a144424f42411a003b567a028201d81858ffd8798c4100581cc3bfbe0930bf8cc56c2d727a10ce373b45f0cc7ef9ac7d8a4366dd47d87982581cabb15dbbcc5c7c80cebea450f4f2131ec1f5b27ca38b66418e4c9a5144424f42411a003b567a1a000927c01a004aefd4d879824040d879821b002cdde00dad87191b002386f26fc100001a0007a120d87982d87981581cb6560f277ce0092e01f39fcf534b2e7a58282260cf044527f1e7f74dd87981d87981d87981581c33b2974f65764819697031e36678e903e97c723c364aa19f8cb5103e581cb6560f277ce0092e01f39fcf534b2e7a58282260cf044527f1e7f74d81581c9beb201348b07d30ee9370b0c353fb0ef566a4c79b153477f15ccf9482583901b6560f277ce0092e01f39fcf534b2e7a58282260cf044527f1e7f74d33b2974f65764819697031e36678e903e97c723c364aa19f8cb5103e1a0030c278021a0002d199a10081825820b1b1f9da358b4e88657bd65bcf3d69bf3f21d37ce4a258e57e0637722510ea5b5840bd03a55b73391847baaf6ff40a86c77b8145b780687d00806047f1434d153b890b27869d6dc9290f57546c001d5b09c1b5061ed612615b0f1ce97c0dc9a3b500f5f6";
    const TX_EXE: &str = "84a8008382582097413f4043c1e2ffeb7c05f098cda6bb465d20db90f4f78fa25c1b64b603bb4a0082582097413f4043c1e2ffeb7c05f098cda6bb465d20db90f4f78fa25c1b64b603bb4a01825820c32020e87e6859df3d29e302ae992ee4742b21da69bf1bbb8d956146fec04a72000183a300583931905ab869961b094f1b8197278cfe15b45cbe49fa8f32c6b014f85a2db2f6abf60ccde92eae1a2f4fdf65f2eaf6208d872c6f0e597cc10b0701821a00f0eb18a2581c63f947b8d9535bc4e4ce6919e3dc056547e8d30ada12f29aa5f826b8a15820588e3c07f5c2119882784077cb720649309bb0a024383d44f5436c9c5c15cbfe01581cabb15dbbcc5c7c80cebea450f4f2131ec1f5b27ca38b66418e4c9a51a144424f42411a3b4e2624028201d81858e2d87989d87982581c63f947b8d9535bc4e4ce6919e3dc056547e8d30ada12f29aa5f826b85820588e3c07f5c2119882784077cb720649309bb0a024383d44f5436c9c5c15cbfed879824040d87982581cabb15dbbcc5c7c80cebea450f4f2131ec1f5b27ca38b66418e4c9a5144424f42411b0000001c871b063f1a0026d61e581c9beb201348b07d30ee9370b0c353fb0ef566a4c79b153477f15ccf941b000000043c4abc40581c8807fbe6e36b1c35ad6f36f0993e2fc67ab6f2db06041cfa3a53c04a581c30c1003aa7dec834e0d0a78db547ba8840e58060725dbfae352f0d64825839019beb201348b07d30ee9370b0c353fb0ef566a4c79b153477f15ccf945df68403295da27216dd1a22809feaa53552e84ae3442efe74d77d851a00885794a200583901b6560f277ce0092e01f39fcf534b2e7a58282260cf044527f1e7f74d33b2974f65764819697031e36678e903e97c723c364aa19f8cb5103e011a00acc30a021a0005ccc905a1581df196f5c1bee23481335ff4aece32fe1dfa1aa40a944a66d2d6edc9a9a5000b582034e73784c72799213d02cad0277814472773c3610fee3bcb7859f14517cff2120d81825820b54aa6b7fa267f7c21cc053b3ecc629ba8ec32e320c65a85d3de327571bad14a040e81581c9beb201348b07d30ee9370b0c353fb0ef566a4c79b153477f15ccf941283825820c4a540ac2e06c217dd4fb3f39ca3863da394ba134677dafa9b98830ca71d584d03825820b91eda29d145ab6c0bc0d6b7093cb24b131440b7b015033205476f39c690a51f00825820b91eda29d145ab6c0bc0d6b7093cb24b131440b7b015033205476f39c690a51f01a200818258208f11dc37d81c0dff768d41bbb1bbc30328283183fd608bcb2eec9ccbafc1c52a5840bfeb0148753db56aeb4d52a43cafd7174002ff43d45265a6e45843ec9801a58dbf9fb9671fedc35624511c697344aa8cd3d8756a48f4c6f990f9ba1394be99050583840000d879830000d87980821a000864701a0d1cef00840002d87a80821a000186a01a01c9c38084030080821a000668a01a09896800f5f6";

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
