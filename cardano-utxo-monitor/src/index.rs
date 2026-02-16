use async_trait::async_trait;
use cml_chain::address::Address;
use cml_chain::certs::Credential;
use cml_chain::transaction::TransactionOutput;
use cml_chain::{Deserialize, Serialize, Slot};
use cml_crypto::{RawBytesEncoding, TransactionHash};
use log::trace;
use rocksdb::{
    ColumnFamily, DBIteratorWithThreadMode, Direction, IteratorMode, Options, ReadOptions,
    SnapshotWithThreadMode, Transaction, TransactionDB, TransactionDBOptions,
};
use spectrum_cardano_lib::transaction::TransactionOutputExtension;
use spectrum_cardano_lib::OutputRef;
use spectrum_offchain::display::{display_option, display_vec};
use spectrum_offchain::tracing::Tracing;
use std::fmt::{Display, Formatter};
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
        settled_at: Option<Slot>,
    );
    async fn unapply(
        &self,
        tx_hash: TransactionHash,
        inputs: Vec<OutputRef>,
        outputs: Vec<(usize, TransactionOutput)>,
    );
}

#[async_trait]
impl<In: UtxoIndex + Sync> UtxoIndex for Tracing<In> {
    async fn apply(
        &self,
        tx_hash: TransactionHash,
        inputs: Vec<OutputRef>,
        outputs: Vec<(usize, TransactionOutput)>,
        confirmed_at: Option<Slot>,
    ) {
        trace!(
            "UtxoIndex::apply(tx_hash={}, inputs={}, outputs={}, confirmed_at={})",
            tx_hash,
            display_vec(&inputs),
            display_vec(&outputs.iter().map(|(i, _)| *i).collect()),
            display_option(&confirmed_at),
        );
        self.component.apply(tx_hash, inputs, outputs, confirmed_at).await;
    }

    async fn unapply(
        &self,
        tx_hash: TransactionHash,
        inputs: Vec<OutputRef>,
        outputs: Vec<(usize, TransactionOutput)>,
    ) {
        trace!(
            "UtxoIndex::unapply(tx_hash={}, inputs={}, outputs={})",
            tx_hash,
            display_vec(&inputs),
            display_vec(&outputs.iter().map(|(i, _)| *i).collect())
        );
        self.component.unapply(tx_hash, inputs, outputs).await;
    }
}

#[derive(Debug, Clone)]
pub struct Txo {
    pub oref: OutputRef,
    pub output: TransactionOutput,
    pub settled_at: Option<Slot>,
    pub spent: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CredentialKind {
    Payment,
    Stake,
}

#[async_trait]
pub trait UtxoResolver {
    async fn get_utxos(
        &self,
        scope: Option<(Credential, CredentialKind)>,
        query: TxoQuery,
        offset: usize,
        limit: usize,
    ) -> Vec<Txo>;
}

#[derive(Debug, Copy, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TxoQuery {
    All(Option<Slot>),
    Unspent,
}

impl Display for TxoQuery {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            TxoQuery::All(slt) => write!(f, "TxoQuery::All({})", display_option(slt)),
            TxoQuery::Unspent => write!(f, "TxoQuery::Unspent"),
        }
    }
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

struct Cols<'a> {
    txo_cf: &'a ColumnFamily,
    payment_events_cf: &'a ColumnFamily,
    payment_unspent_cf: &'a ColumnFamily,
    stake_events_cf: &'a ColumnFamily,
    stake_unspent_cf: &'a ColumnFamily,
}

impl<'a> Cols<'a> {
    fn index_cfs(&self, kind: CredentialKind) -> (&'a ColumnFamily, &'a ColumnFamily) {
        match kind {
            CredentialKind::Payment => (self.payment_events_cf, self.payment_unspent_cf),
            CredentialKind::Stake => (self.stake_events_cf, self.stake_unspent_cf),
        }
    }
}

fn get_columns(db: &Arc<TransactionDB>) -> Cols<'_> {
    let txo_cf = db.cf_handle(TABLES[0]).unwrap();
    let payment_events_cf = db.cf_handle(TABLES[1]).unwrap();
    let payment_unspent_cf = db.cf_handle(TABLES[2]).unwrap();
    let stake_events_cf = db.cf_handle(TABLES[3]).unwrap();
    let stake_unspent_cf = db.cf_handle(TABLES[4]).unwrap();
    Cols {
        txo_cf,
        payment_events_cf,
        payment_unspent_cf,
        stake_events_cf,
        stake_unspent_cf,
    }
}

const TABLES: [&str; 5] = [
    "txo",
    "pkh_to_txo_events",
    "pkh_to_txo_unspent",
    "stake_cred_to_txo_events",
    "stake_cred_to_txo_unspent",
];

fn utxo_key(rf: OutputRef) -> Vec<u8> {
    rmp_serde::to_vec(&rf).unwrap()
}

fn credential_tag_and_bytes(credential: &Credential) -> (u8, Vec<u8>) {
    match credential {
        Credential::PubKey { hash, .. } => (0, hash.to_raw_bytes().to_vec()),
        Credential::Script { hash, .. } => (1, hash.to_raw_bytes().to_vec()),
    }
}

fn credential_key_prefix(credential: &Credential) -> Vec<u8> {
    let (tag, bytes) = credential_tag_and_bytes(credential);
    let mut prefix = Vec::with_capacity(1 + bytes.len());
    prefix.push(tag);
    prefix.extend(bytes);
    prefix
}

fn settled_at_key(settled_at: Option<Slot>) -> Vec<u8> {
    settled_at.unwrap_or(u64::MAX).to_be_bytes().to_vec()
}

fn credential_to_utxo_all_key(credential: &Credential, settled_at: Option<Slot>, rf: OutputRef) -> Vec<u8> {
    let mut key = credential_key_prefix(credential);
    key.extend(settled_at_key(settled_at));
    key.extend(utxo_key(rf));
    key
}

fn credential_to_utxo_unspent_key(credential: &Credential, rf: OutputRef) -> Vec<u8> {
    let mut key = credential_key_prefix(credential);
    key.extend(utxo_key(rf));
    key
}

const UNIT_INDEX_SENTINEL: u8 = 0;

fn unit_index_key_prefix(unit: &str) -> Vec<u8> {
    unit.as_bytes().to_vec()
}

fn unit_index_entry_key(prefix: &[u8], oref: OutputRef) -> Vec<u8> {
    let mut key = Vec::with_capacity(prefix.len() + 16);
    key.extend_from_slice(prefix);
    key.extend(utxo_key(oref));
    key
}

fn unit_index_sentinel_key(prefix: &[u8]) -> Vec<u8> {
    let mut key = Vec::with_capacity(prefix.len() + 1);
    key.extend_from_slice(prefix);
    key.push(UNIT_INDEX_SENTINEL);
    key
}

fn output_units(output: &TransactionOutput) -> Vec<String> {
    output
        .value()
        .multiasset
        .iter()
        .flat_map(|(policy_id, assets)| {
            let policy_hex = policy_id.to_string();
            assets.iter().map(move |(asset_name, _)| {
                let asset_hex = asset_name.to_raw_hex();
                let mut combined = String::with_capacity(policy_hex.len() + asset_hex.len());
                combined.push_str(&policy_hex);
                combined.push_str(&asset_hex);
                combined.make_ascii_lowercase();
                combined
            })
        })
        .collect()
}

fn write_unit_indexes(
    tx: &Transaction<TransactionDB>,
    unit_cf: &ColumnFamily,
    output: &TransactionOutput,
    oref: OutputRef,
) {
    for unit in output_units(output) {
        let prefix = unit_index_key_prefix(&unit);
        let sentinel = unit_index_sentinel_key(&prefix);
        let _ = tx.delete_cf(unit_cf, sentinel);
        let key = unit_index_entry_key(&prefix, oref);
        tx.put_cf(unit_cf, key, vec![]).unwrap();
    }
}

fn delete_unit_indexes(
    tx: &Transaction<TransactionDB>,
    unit_cf: &ColumnFamily,
    output: &TransactionOutput,
    oref: OutputRef,
) {
    for unit in output_units(output) {
        let prefix = unit_index_key_prefix(&unit);
        let key = unit_index_entry_key(&prefix, oref);
        let _ = tx.delete_cf(unit_cf, key);
    }
}

fn get_utxo_by_ref(
    tx: &Transaction<TransactionDB>,
    utxos: &ColumnFamily,
    output_ref: OutputRef,
) -> Option<(TransactionOutput, Option<Slot>, bool)> {
    tx.get_cf(utxos, utxo_key(output_ref))
        .unwrap()
        .and_then(|bytes| rmp_serde::from_slice::<(Vec<u8>, Option<Slot>, bool)>(&bytes).ok())
        .and_then(|(utxo_bytes, settled_at, spent)| {
            TransactionOutput::from_cbor_bytes(&utxo_bytes)
                .ok()
                .map(|o| (o, settled_at, spent))
        })
}

fn write_txo(
    tx: &Transaction<TransactionDB>,
    txo_cf: &ColumnFamily,
    oref: OutputRef,
    txo: TransactionOutput,
    settled_at: Option<Slot>,
    spent: bool,
) {
    let bytes = rmp_serde::to_vec(&(txo.to_canonical_cbor_bytes(), settled_at, spent)).unwrap();
    tx.put_cf(txo_cf, utxo_key(oref), bytes).unwrap();
}

fn write_secondary_indexes(
    tx: &Transaction<TransactionDB>,
    events_cf: &ColumnFamily,
    unspent_cf: &ColumnFamily,
    credential: &Credential,
    oref: OutputRef,
    slot: Option<Slot>,
    spent: bool,
) {
    let all_index_key = credential_to_utxo_all_key(credential, slot, oref);
    tx.put_cf(events_cf, all_index_key, vec![]).unwrap();
    if !spent {
        let unspent_index_key = credential_to_utxo_unspent_key(credential, oref);
        tx.put_cf(unspent_cf, unspent_index_key, vec![]).unwrap();
    }
}

fn address_credentials(address: &Address) -> Vec<(CredentialKind, Credential)> {
    let mut credentials = Vec::with_capacity(2);
    match address {
        Address::Base(base) => {
            credentials.push((CredentialKind::Payment, base.payment.clone()));
            credentials.push((CredentialKind::Stake, base.stake.clone()));
        }
        Address::Enterprise(enterprise) => {
            credentials.push((CredentialKind::Payment, enterprise.payment.clone()));
        }
        Address::Ptr(ptr) => {
            credentials.push((CredentialKind::Payment, ptr.payment.clone()));
        }
        Address::Reward(reward) => {
            credentials.push((CredentialKind::Stake, reward.payment.clone()));
        }
        Address::Byron(_) => {}
    }
    credentials
}

fn update_unspent_txo(
    tx: &Transaction<TransactionDB>,
    cols: &Cols,
    oref: OutputRef,
    txo: TransactionOutput,
    settled_at: Option<Slot>,
) {
    let credentials = address_credentials(txo.address());
    let mut spent = false;

    if let Some((existing_out, existing_settled_at, existing_spent)) = get_utxo_by_ref(tx, cols.txo_cf, oref)
    {
        trace!(
            "Updating txo {} from confirmation, preserving spent: {}",
            oref,
            existing_spent
        );
        let existing_credentials = address_credentials(existing_out.address());
        for (kind, credential) in &existing_credentials {
            let (events_cf, unspent_cf) = cols.index_cfs(*kind);
            delete_indexes(tx, events_cf, unspent_cf, credential, existing_settled_at, oref);
        }
        tx.delete_cf(cols.txo_cf, utxo_key(oref)).unwrap();
        spent = existing_spent;
    }

    for (kind, credential) in &credentials {
        let (events_cf, unspent_cf) = cols.index_cfs(*kind);
        write_secondary_indexes(tx, events_cf, unspent_cf, credential, oref, settled_at, spent);
    }
    write_txo(tx, cols.txo_cf, oref, txo, settled_at, spent);
}

fn update_txo_by_ref(
    tx: &Transaction<TransactionDB>,
    cols: &Cols,
    oref: OutputRef,
    slot: Option<Slot>,
    spent: bool,
) {
    if let Some((out, settled_at, already_spent)) = get_utxo_by_ref(tx, cols.txo_cf, oref) {
        trace!("Updating txo {}, spent: {} => {}", oref, already_spent, spent);
        let credentials = address_credentials(out.address());
        for (kind, credential) in &credentials {
            let (events_cf, unspent_cf) = cols.index_cfs(*kind);
            delete_indexes(tx, events_cf, unspent_cf, credential, settled_at, oref);
            let (index_slot, index_spent) = if spent { (slot, true) } else { (settled_at, false) };
            write_secondary_indexes(
                tx,
                events_cf,
                unspent_cf,
                credential,
                oref,
                index_slot,
                index_spent,
            );
        }
        write_txo(tx, cols.txo_cf, oref, out, settled_at, spent);
    }
}

fn delete_indexes(
    tx: &Transaction<TransactionDB>,
    events_cf: &ColumnFamily,
    unspent_cf: &ColumnFamily,
    credential: &Credential,
    settled_at: Option<Slot>,
    oref: OutputRef,
) {
    tx.delete_cf(
        events_cf,
        credential_to_utxo_all_key(credential, settled_at, oref),
    )
    .unwrap();
    tx.delete_cf(unspent_cf, credential_to_utxo_unspent_key(credential, oref))
        .unwrap();
}

fn delete_txo_and_indexes(tx: &Transaction<TransactionDB>, cols: &Cols, oref: OutputRef) {
    if let Some((out, settled_at, _spent)) = get_utxo_by_ref(tx, cols.txo_cf, oref) {
        trace!("Deleting txo {}", oref);
        let credentials = address_credentials(out.address());
        for (kind, credential) in &credentials {
            let (events_cf, unspent_cf) = cols.index_cfs(*kind);
            delete_indexes(tx, events_cf, unspent_cf, credential, settled_at, oref);
        }
        tx.delete_cf(cols.txo_cf, utxo_key(oref)).unwrap();
    }
}

#[async_trait]
impl UtxoIndex for RocksDB {
    async fn apply(
        &self,
        tx_hash: TransactionHash,
        inputs: Vec<OutputRef>,
        outputs: Vec<(usize, TransactionOutput)>,
        confirmed_at: Option<Slot>,
    ) {
        let db = self.db.clone();
        spawn_blocking(move || {
            let cols = get_columns(&db);
            let tx = db.transaction();
            for oref in inputs {
                update_txo_by_ref(&tx, &cols, oref, confirmed_at, true);
            }
            for (ix, o) in outputs {
                let rf = OutputRef::new(tx_hash, ix as u64);
                update_unspent_txo(&tx, &cols, rf, o, confirmed_at);
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
            let cols = get_columns(&db);
            let tx = db.transaction();
            for oref in inputs {
                update_txo_by_ref(&tx, &cols, oref, None, false);
            }
            for (ix, _o) in outputs {
                let rf = OutputRef::new(tx_hash, ix as u64);
                delete_txo_and_indexes(&tx, &cols, rf);
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
        scope: Option<(Credential, CredentialKind)>,
        query: TxoQuery,
        offset: usize,
        limit: usize,
    ) -> Vec<Txo> {
        let db = self.db.clone();
        spawn_blocking(move || {
            if limit == 0 {
                return Vec::new();
            }

            let cols = get_columns(&db);
            let snap = db.snapshot();

            match (scope, query) {
                (Some((credential, kind)), query_variant) => {
                    let (events_cf, unspent_cf) = cols.index_cfs(kind);
                    let prefix = credential_key_prefix(&credential);
                    let prefix_len = prefix.len();
                    let (num_key_bytes_to_drop, index_cf, lower_bound) = match query_variant {
                        TxoQuery::All(least_slot) => {
                            (prefix_len + 8, events_cf, Some(settled_at_key(least_slot)))
                        }
                        TxoQuery::Unspent => (prefix_len, unspent_cf, None),
                    };
                    let mut txo_iter = get_range_iterator(&snap, index_cf, prefix.clone(), lower_bound);
                    let mut txo_set = vec![];
                    let mut matched = 0usize;
                    while let Some(Ok((index, _))) = txo_iter.next() {
                        let utxo_key = &index[num_key_bytes_to_drop..];
                        if let Some((output, confirmed_at, spent)) = snap
                            .get_cf(cols.txo_cf, utxo_key)
                            .unwrap()
                            .and_then(|bytes| {
                                rmp_serde::from_slice::<(Vec<u8>, Option<Slot>, bool)>(&bytes).ok()
                            })
                            .and_then(|(utxo_bytes, settled_at, spent)| {
                                TransactionOutput::from_cbor_bytes(&utxo_bytes)
                                    .ok()
                                    .map(|o| (o, settled_at, spent))
                            })
                        {
                            let oref = rmp_serde::from_slice::<OutputRef>(utxo_key).unwrap();
                            if matched < offset {
                                matched += 1;
                                continue;
                            }
                            txo_set.push(Txo {
                                oref,
                                output,
                                settled_at: confirmed_at,
                                spent,
                            });
                            matched += 1;
                            if txo_set.len() >= limit {
                                break;
                            }
                        }
                    }
                    txo_set
                }
                (None, _) => Vec::new(),
            }
        })
        .await
        .unwrap()
    }
}

fn output_contains_unit(output: &TransactionOutput, unit: &str) -> bool {
    let unit_lower = unit.to_ascii_lowercase();
    output_units(output)
        .into_iter()
        .any(|candidate| candidate == unit_lower)
}

pub(crate) fn get_range_iterator<'a: 'b, 'b>(
    db: &'a SnapshotWithThreadMode<'b, TransactionDB>,
    cf: &ColumnFamily,
    prefix: Vec<u8>,
    lower_bound: Option<Vec<u8>>,
) -> DBIteratorWithThreadMode<'b, TransactionDB> {
    let mut readopts = ReadOptions::default();
    let from = if let Some(lower_bound) = lower_bound {
        prefix.clone().into_iter().chain(lower_bound).collect::<Vec<_>>()
    } else {
        prefix.clone()
    };
    readopts.set_iterate_range(rocksdb::PrefixRange(prefix));
    db.iterator_cf_opt(cf, readopts, IteratorMode::From(&from, Direction::Forward))
}

#[cfg(test)]
mod tests {
    use crate::index::{CredentialKind, RocksDB, Txo, TxoQuery, UtxoIndex, UtxoResolver};
    use cml_chain::certs::Credential;
    use cml_chain::transaction::Transaction;
    use cml_chain::{Deserialize, Slot};
    use cml_crypto::Ed25519KeyHash;
    use rocksdb::{Options, SingleThreaded, TransactionDB};
    use spectrum_cardano_lib::transaction::TransactionOutputExtension;
    use spectrum_cardano_lib::OutputRef;
    use spectrum_offchain::tx_hash::CanonicalHash;
    use std::path::{Path, PathBuf};

    #[tokio::test]
    async fn index_applied_transactions_include_spent_mempool() {
        let must_consume_utxo = OutputRef::from_string_unsafe(
            "13de3390f33b18faaeeb91eafc839e28c687f47f146e9c68779562a8a5385afc#0",
        );
        let txos = test_utxo_resolving(
            vec![(TX_PRODUCE, None), (TX_CONSUME, None)],
            "bed3c3bac9ddc7952cc91cf76db3dd808f99f4a0dd07e78e06657bc2",
            CredentialKind::Payment,
            TxoQuery::All(Some(0)),
        )
        .await;
        println!("{:?}", txos.iter().map(|x| (x.oref, x.spent)).collect::<Vec<_>>());
        assert!(txos
            .iter()
            .find(|e| e.oref == must_consume_utxo)
            .map(|txo| txo.spent)
            .unwrap())
    }

    #[tokio::test]
    async fn index_applied_transactions_include_spent_ledger() {
        let must_consume_utxo = OutputRef::from_string_unsafe(
            "13de3390f33b18faaeeb91eafc839e28c687f47f146e9c68779562a8a5385afc#0",
        );
        let txos = test_utxo_resolving(
            vec![(TX_PRODUCE, Some(1)), (TX_CONSUME, None)],
            "bed3c3bac9ddc7952cc91cf76db3dd808f99f4a0dd07e78e06657bc2",
            CredentialKind::Payment,
            TxoQuery::All(Some(0)),
        )
        .await;
        println!("{:?}", txos.iter().map(|x| (x.oref, x.spent)).collect::<Vec<_>>());
        assert!(txos
            .iter()
            .find(|e| e.oref == must_consume_utxo)
            .map(|txo| txo.spent)
            .unwrap())
    }

    #[tokio::test]
    async fn confirmation_preserves_pending_spend_state() {
        let db_path = DBPath::new("_index_confirm_preserves_pending");
        let db = RocksDB::new(&db_path);

        let produce_tx = Transaction::from_cbor_bytes(&*hex::decode(TX_PRODUCE).unwrap()).unwrap();
        let produce_hash = produce_tx.canonical_hash();
        let produce_inputs: Vec<_> = produce_tx
            .body
            .inputs
            .clone()
            .into_iter()
            .map(|i| i.into())
            .collect();
        let produce_outputs_for_db: Vec<_> =
            produce_tx.body.outputs.to_vec().into_iter().enumerate().collect();
        db.apply(produce_hash, produce_inputs, produce_outputs_for_db, None)
            .await;

        let consume_tx = Transaction::from_cbor_bytes(&*hex::decode(TX_CONSUME).unwrap()).unwrap();
        let consume_hash = consume_tx.canonical_hash();
        let consume_inputs: Vec<_> = consume_tx
            .body
            .inputs
            .clone()
            .into_iter()
            .map(|i| i.into())
            .collect();
        let consume_outputs_for_db: Vec<_> =
            consume_tx.body.outputs.to_vec().into_iter().enumerate().collect();
        db.apply(consume_hash, consume_inputs, consume_outputs_for_db, None)
            .await;

        let produce_tx_confirmed = Transaction::from_cbor_bytes(&*hex::decode(TX_PRODUCE).unwrap()).unwrap();
        let produce_inputs_confirmed: Vec<_> = produce_tx_confirmed
            .body
            .inputs
            .clone()
            .into_iter()
            .map(|i| i.into())
            .collect();
        let produce_outputs_confirmed: Vec<_> = produce_tx_confirmed
            .body
            .outputs
            .to_vec()
            .into_iter()
            .enumerate()
            .collect();
        db.apply(
            produce_tx_confirmed.canonical_hash(),
            produce_inputs_confirmed,
            produce_outputs_confirmed,
            Some(1),
        )
        .await;

        let credential = Credential::new_pub_key(
            Ed25519KeyHash::from_hex("bed3c3bac9ddc7952cc91cf76db3dd808f99f4a0dd07e78e06657bc2").unwrap(),
        );
        let target_oref = OutputRef::from_string_unsafe(
            "13de3390f33b18faaeeb91eafc839e28c687f47f146e9c68779562a8a5385afc#0",
        );

        let all = db
            .get_utxos(
                Some((credential.clone(), CredentialKind::Payment)),
                TxoQuery::All(Some(0)),
                0,
                10,
            )
            .await;
        let entry = all
            .iter()
            .find(|txo| txo.oref == target_oref)
            .expect("pending spend should remain tracked");
        assert!(entry.spent);

        let unspent = db
            .get_utxos(
                Some((credential, CredentialKind::Payment)),
                TxoQuery::Unspent,
                0,
                10,
            )
            .await;
        assert!(unspent.iter().all(|txo| txo.oref != target_oref));
    }

    #[tokio::test]
    async fn index_applied_transactions() {
        let must_consume_utxo = OutputRef::from_string_unsafe(
            "13de3390f33b18faaeeb91eafc839e28c687f47f146e9c68779562a8a5385afc#0",
        );
        let txos = test_utxo_resolving(
            vec![(TX_PRODUCE, Some(1)), (TX_CONSUME, None)],
            "bed3c3bac9ddc7952cc91cf76db3dd808f99f4a0dd07e78e06657bc2",
            CredentialKind::Payment,
            TxoQuery::Unspent,
        )
        .await;
        assert!(txos.iter().find(|e| e.oref == must_consume_utxo).is_none())
    }

    #[tokio::test]
    async fn index_supports_stake_credentials() {
        let db_path = DBPath::new("_index_stake_credentials");
        let db = RocksDB::new(&db_path);
        let tx_bytes = hex::decode(TX_PRODUCE).unwrap();
        let tx = Transaction::from_cbor_bytes(&tx_bytes).unwrap();
        let hash = tx.canonical_hash();
        let inputs: Vec<_> = tx.body.inputs.clone().into_iter().map(|i| i.into()).collect();
        let outputs_vec = tx.body.outputs.to_vec();
        let outputs_for_db: Vec<_> = outputs_vec.clone().into_iter().enumerate().collect();
        db.apply(hash, inputs, outputs_for_db, Some(1)).await;

        let mut stake_credential = None;
        let mut payment_credential = None;
        let mut expected_oref = None;
        for (ix, output) in outputs_vec.iter().enumerate() {
            for (kind, credential) in super::address_credentials(output.address()) {
                match kind {
                    CredentialKind::Payment => {
                        if payment_credential.is_none() {
                            payment_credential = Some(credential);
                        }
                    }
                    CredentialKind::Stake => {
                        stake_credential = Some(credential);
                        expected_oref = Some(OutputRef::new(hash, ix as u64));
                        break;
                    }
                }
            }
            if stake_credential.is_some() {
                break;
            }
        }

        let stake_credential = stake_credential.expect("stake credential found");
        let expected_oref = expected_oref.expect("expected oref");
        let stake_utxos = db
            .get_utxos(
                Some((stake_credential.clone(), CredentialKind::Stake)),
                TxoQuery::All(Some(0)),
                0,
                100,
            )
            .await;
        assert!(stake_utxos.iter().any(|txo| txo.oref == expected_oref));

        if let Some(payment_credential) = payment_credential {
            let payment_utxos = db
                .get_utxos(
                    Some((payment_credential, CredentialKind::Payment)),
                    TxoQuery::All(Some(0)),
                    0,
                    100,
                )
                .await;
            assert!(payment_utxos.iter().any(|txo| txo.oref == expected_oref));
        }
    }

    #[tokio::test]
    async fn txos_by_least_event_slot() {
        let _all_txos = test_utxo_resolving(
            vec![
                (TX_FUN_1, Some(1)),
                (TX_FUN_2, Some(2)),
                (TX_ORD, Some(3)),
                (TX_EXE, Some(4)),
            ],
            "b6560f277ce0092e01f39fcf534b2e7a58282260cf044527f1e7f74d",
            CredentialKind::Payment,
            TxoQuery::All(Some(0)),
        )
        .await;
        let lb_slot = 4;
        let tail_txos = test_utxo_resolving(
            vec![
                (TX_FUN_1, Some(1)),
                (TX_FUN_2, Some(2)),
                (TX_ORD, Some(3)),
                (TX_EXE, Some(4)),
            ],
            "b6560f277ce0092e01f39fcf534b2e7a58282260cf044527f1e7f74d",
            CredentialKind::Payment,
            TxoQuery::All(Some(lb_slot)),
        )
        .await;
        assert_eq!(
            tail_txos
                .iter()
                .filter(|x| match x.settled_at {
                    None => true,
                    Some(settled_at) => settled_at >= lb_slot,
                })
                .collect::<Vec<_>>()
                .len(),
            tail_txos.len()
        );
    }

    async fn test_utxo_resolving(
        txs: Vec<(&str, Option<Slot>)>,
        credential_hex: &str,
        kind: CredentialKind,
        q: TxoQuery,
    ) -> Vec<Txo> {
        let db_path = DBPath::new("_index_applied_transactions");
        let db = RocksDB::new(&db_path);
        let credential = Credential::new_pub_key(Ed25519KeyHash::from_hex(credential_hex).unwrap());
        for (rtx, settled_at) in txs {
            let tx = Transaction::from_cbor_bytes(&*hex::decode(rtx).unwrap()).unwrap();
            let hash = tx.canonical_hash();
            db.apply(
                hash,
                tx.body.inputs.into_iter().map(|i| i.into()).collect(),
                tx.body.outputs.to_vec().into_iter().enumerate().collect(),
                settled_at,
            )
            .await;
        }
        db.get_utxos(Some((credential, kind)), q, 0, 100).await
    }

    const TX_PRODUCE: &str = "84a7008182582086ecf8a72d7d1744deefc1c923c7f1ed8eb09a549cb70fde3d572316e066bfa403018182583901bed3c3bac9ddc7952cc91cf76db3dd808f99f4a0dd07e78e06657bc21cc69f513f9551f517c5212855ece1b34d0128f9f9b54e47cebadd4b821b0000000103f98686ad581c0ece814aa1cc2c98981c7690083dbcb51c5bb1279ae408873d8c8762a15820595479793659676e546b3069574c396a4544315943352f49516873337252343701581c15509d4cb60f066ca4c7e982d764d6ceb4324cb33776d1711da1beeea24e42616279416c69656e3034373231014e42616279416c69656e303831313801581c279c909f348e533da5808898f87f9a14bb2c3dfbbacccd631d927a3fa144534e454b19a839581c29d222ce763455e3d7a09a665ce554f00ac89d2e99a1a83d267170c6a1434d494e194a31581c51a5e236c4de3af2b8020442e2a26f454fda3b04cb621c1294a0ef34a144424f4f4b1a0165f8a0581c530a197fe7c275f204c3396b3782fc738f4968f0c81dd2291cf07b8aa3581a434330303337303030303030303030303030303135323030303001581a434330303337303030303030303030303030303135323030363401581a434330313531303030303030303030303030303137363030363401581c5ee425062d88069b702a38a357895132b9b50c8f893c8cf87a4c8c32a14445574d5401581ca0028f350aaabe0545fdcb56b039bfb08e4bb4d8c4d7c3c7d481c235a145484f534b591a04277dbf581ca7904896a247d3aa09478e856769b82d1f2e060028b6bda5543b699fa64d4343434f4c4c41423030303337014d4343434f4c4c41423030313531014d4343434f4c4c41423038393235014d4343434f4c4c4142303930333901581c4375746543726561747572657343686164694e61737361723030333701581c4375746543726561747572657343686164694e61737361723031353101581ce5a42a1a1d3d1da71b0449663c32798725888d2eb0843c4dabeca05aa151576f726c644d6f62696c65546f6b656e581a000f4240581cecbe846aa1a535579d67f9480fa6173b64d7e239df0460eba36e3ad0a14a0014df1053617475726e1a000f4240581cf0ff48bbb7bbe9d59a40f1ce90e9e9d0ff5002ec48f232b49ca0fb9aa14b736f667462696e61746f7201581cfe38ef97888dfde0292b7d2ed103543ecf92a419a29634f513a1d71fa14541534e454b1a00249f00021a00033cd9031a0936d6fe048183028200581c1cc69f513f9551f517c5212855ece1b34d0128f9f9b54e47cebadd4b581c538299a358e79a289c8de779f8cd09dd6a6bb286de717d1f744bb35705a1581de11cc69f513f9551f517c5212855ece1b34d0128f9f9b54e47cebadd4b1a002bdbc50758201c45c96126112c50a121c7bce6fa0b0fb8e7fa6990d0a9435a89a91a1460fccea1008282582061b2624741ddbcd41a6e3490b2e4a71bcc1cb6ff891137097540ad7fa8cf56115840a00c437ed8a787f4fbdd39ada04c3cf6191ee26abe44137a6eb4a2dcf55e4387bfbb565133d42df61f0ed45de43adeec019c2f8c5e0fae209534a266755b96038258201b775e64b1cb83f9420829d807b892da0c8c2ea89873028c620b8c04eb35ee4558400d97db78925083b9736b66ccf28593638cba828f5806311eb6d4aa7d861a53023a2ab27a5f56d65beafb177f756e746f98e093a8c56c5982667e4f2f5abf8403f5a11902a2a1636d736781781956455350523a20506172746e65722044656c65676174696f6e";
    const TX_CONSUME: &str = "84a6008182582013de3390f33b18faaeeb91eafc839e28c687f47f146e9c68779562a8a5385afc00018182583901bed3c3bac9ddc7952cc91cf76db3dd808f99f4a0dd07e78e06657bc21cc69f513f9551f517c5212855ece1b34d0128f9f9b54e47cebadd4b821b0000000103f65035ad581c0ece814aa1cc2c98981c7690083dbcb51c5bb1279ae408873d8c8762a15820595479793659676e546b3069574c396a4544315943352f49516873337252343701581c15509d4cb60f066ca4c7e982d764d6ceb4324cb33776d1711da1beeea24e42616279416c69656e3034373231014e42616279416c69656e303831313801581c279c909f348e533da5808898f87f9a14bb2c3dfbbacccd631d927a3fa144534e454b19a839581c29d222ce763455e3d7a09a665ce554f00ac89d2e99a1a83d267170c6a1434d494e194a31581c51a5e236c4de3af2b8020442e2a26f454fda3b04cb621c1294a0ef34a144424f4f4b1a0165f8a0581c530a197fe7c275f204c3396b3782fc738f4968f0c81dd2291cf07b8aa3581a434330303337303030303030303030303030303135323030303001581a434330303337303030303030303030303030303135323030363401581a434330313531303030303030303030303030303137363030363401581c5ee425062d88069b702a38a357895132b9b50c8f893c8cf87a4c8c32a14445574d5401581ca0028f350aaabe0545fdcb56b039bfb08e4bb4d8c4d7c3c7d481c235a145484f534b591a04277dbf581ca7904896a247d3aa09478e856769b82d1f2e060028b6bda5543b699fa64d4343434f4c4c41423030303337014d4343434f4c4c41423030313531014d4343434f4c4c41423038393235014d4343434f4c4c4142303930333901581c4375746543726561747572657343686164694e61737361723030333701581c4375746543726561747572657343686164694e61737361723031353101581ce5a42a1a1d3d1da71b0449663c32798725888d2eb0843c4dabeca05aa151576f726c644d6f62696c65546f6b656e581a000f4240581cecbe846aa1a535579d67f9480fa6173b64d7e239df0460eba36e3ad0a14a0014df1053617475726e1a000f4240581cf0ff48bbb7bbe9d59a40f1ce90e9e9d0ff5002ec48f232b49ca0fb9aa14b736f667462696e61746f7201581cfe38ef97888dfde0292b7d2ed103543ecf92a419a29634f513a1d71fa14541534e454b1a00249f00021a00033651031a0936d713048183028200581c1cc69f513f9551f517c5212855ece1b34d0128f9f9b54e47cebadd4b581cf423b19715cca49029ed13ff02a110b63de7d96ad7a0536dc5887a410758201c45c96126112c50a121c7bce6fa0b0fb8e7fa6990d0a9435a89a91a1460fccea1008282582061b2624741ddbcd41a6e3490b2e4a71bcc1cb6ff891137097540ad7fa8cf56115840b38ff68cbbbfcd3f9c17c968f584baa09b616321ed9cb28cfa1f6a58b39065f538968c87381b1da31692552cf5870fd85ff3e4739c0486046e5410a406222f0a8258201b775e64b1cb83f9420829d807b892da0c8c2ea89873028c620b8c04eb35ee4558409eb2d130f64a4e3338c73ac56d17a553ea8d4d7a796ba00dc08c068946999cefdccd3a699a5e6d464422d33ce483842bb63b5774036c73b10998762ecb675b0af5a11902a2a1636d736781781956455350523a20506172746e65722044656c65676174696f6e";

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
