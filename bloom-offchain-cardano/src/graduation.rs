use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::{Arc, RwLock};
use std::{io, str};

use bounded_integer::BoundedU64;
use cml_chain::certs::StakeCredential;
use cml_chain::transaction::TransactionOutput;
use cml_crypto::{ScriptHash, TransactionHash};
use rocksdb::{IteratorMode, OptimisticTransactionDB};
use spectrum_cardano_lib::plutus_data::DatumExtension;
use spectrum_cardano_lib::types::TryFromPData;
use spectrum_cardano_lib::{OutputRef, Token};
use spectrum_offchain_cardano::data::quadratic_pool::{QuadraticPoolConfig, QuadraticPoolT2TConfig};
use spectrum_offchain_cardano::data::PoolId;

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum PoolOrigin {
    Direct,
    SnekGraduated,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct GraduatedPoolFeeConfig {
    pub enabled: bool,
    pub relative_fee_percent: BoundedU64<0, 99>,
}

impl Default for GraduatedPoolFeeConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            relative_fee_percent: BoundedU64::new_saturating(0),
        }
    }
}

impl GraduatedPoolFeeConfig {
    pub fn try_enabled(relative_fee_percent: u64) -> Result<Self, String> {
        if relative_fee_percent > 99 {
            return Err("relativeFeePercent must be between 0 and 99".to_string());
        }
        Ok(Self {
            enabled: true,
            relative_fee_percent: BoundedU64::new_saturating(relative_fee_percent),
        })
    }

    pub fn enabled(relative_fee_percent: u64) -> Self {
        Self::try_enabled(relative_fee_percent).expect("relativeFeePercent must be between 0 and 99")
    }

    pub fn fee(&self, amount: u64) -> u64 {
        if self.enabled {
            ((amount as u128 * self.relative_fee_percent.get() as u128) / 100) as u64
        } else {
            0
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct GraduatedSplashPoolStore {
    inner: Arc<RwLock<HashSet<Token>>>,
    journal: Arc<RwLock<HashMap<TransactionHash, GraduationJournalEntry>>>,
}

impl GraduatedSplashPoolStore {
    pub fn contains(&self, pool_id: Token) -> bool {
        self.inner
            .read()
            .expect("graduated pool store poisoned")
            .contains(&pool_id)
    }

    pub fn insert(&self, pool_id: Token) {
        self.inner
            .write()
            .expect("graduated pool store poisoned")
            .insert(pool_id);
    }

    pub fn remove(&self, pool_id: Token) {
        self.inner
            .write()
            .expect("graduated pool store poisoned")
            .remove(&pool_id);
    }

    pub fn extend<I: IntoIterator<Item = Token>>(&self, ids: I) {
        self.inner
            .write()
            .expect("graduated pool store poisoned")
            .extend(ids);
    }

    pub fn journal_applied(&self, tx_hash: TransactionHash, entry: GraduationJournalEntry) {
        self.journal
            .write()
            .expect("graduation journal poisoned")
            .insert(tx_hash, entry);
    }

    pub fn apply_observation(
        &self,
        tx_hash: TransactionHash,
        tracker: &SnekPoolInputTracker,
        consumed_snek_refs: Vec<(OutputRef, Token)>,
        produced_snek_refs: Vec<(OutputRef, Token)>,
        graduated_splash_ids: Vec<Token>,
    ) -> GraduationJournalEntry {
        let entry = GraduationJournalEntry {
            consumed_snek_refs,
            produced_snek_refs,
            graduated_splash_ids,
        };
        self.apply_observation_entry(tx_hash, tracker, entry.clone());
        entry
    }

    pub fn apply_observation_entry(
        &self,
        tx_hash: TransactionHash,
        tracker: &SnekPoolInputTracker,
        entry: GraduationJournalEntry,
    ) {
        for (output_ref, _) in &entry.consumed_snek_refs {
            tracker.remove(*output_ref);
        }
        for (output_ref, pool_id) in &entry.produced_snek_refs {
            tracker.insert(*output_ref, *pool_id);
        }
        self.extend(entry.graduated_splash_ids.iter().copied());
        self.journal_applied(tx_hash, entry);
    }

    pub fn rollback_tx(
        &self,
        tx_hash: TransactionHash,
        tracker: &SnekPoolInputTracker,
    ) -> Option<GraduationJournalEntry> {
        let entry = self
            .journal
            .write()
            .expect("graduation journal poisoned")
            .remove(&tx_hash)?;
        tracker.rollback_tx(entry.clone(), self);
        Some(entry)
    }

    pub fn snapshot(&self) -> Vec<Token> {
        self.inner
            .read()
            .expect("graduated pool store poisoned")
            .iter()
            .copied()
            .collect()
    }
}

#[derive(Clone)]
pub struct GraduationStateRocksDb {
    db: Arc<OptimisticTransactionDB>,
}

impl std::fmt::Debug for GraduationStateRocksDb {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GraduationStateRocksDb").finish_non_exhaustive()
    }
}

const GRADUATED_POOL_KEY_PREFIX: &[u8] = b"graduated:";
const SNEK_POOL_INPUT_KEY_PREFIX: &[u8] = b"snek-input:";
const JOURNAL_KEY_PREFIX: &[u8] = b"journal:";

impl GraduationStateRocksDb {
    pub fn open(path: &Path) -> io::Result<Self> {
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)?;
            }
        }
        OptimisticTransactionDB::open_default(path)
            .map(|db| Self { db: Arc::new(db) })
            .map_err(rocksdb_error)
    }

    pub fn load_state(&self) -> io::Result<(GraduatedSplashPoolStore, SnekPoolInputTracker)> {
        let store = GraduatedSplashPoolStore::default();
        let tracker = SnekPoolInputTracker::default();

        for item in self.db.iterator(IteratorMode::Start) {
            let (key, value) = item.map_err(rocksdb_error)?;
            if let Some(raw_pool_id) = key.strip_prefix(GRADUATED_POOL_KEY_PREFIX) {
                store.insert(parse_token(raw_pool_id)?);
            } else if let Some(raw_output_ref) = key.strip_prefix(SNEK_POOL_INPUT_KEY_PREFIX) {
                let output_ref = parse_output_ref(raw_output_ref)?;
                let pool_id = parse_token(&value)?;
                tracker.insert(output_ref, pool_id);
            } else if let Some(raw_tx_hash) = key.strip_prefix(JOURNAL_KEY_PREFIX) {
                let tx_hash = parse_tx_hash(raw_tx_hash)?;
                let entry = parse_journal_entry(&value)?;
                store.journal_applied(tx_hash, entry);
            }
        }

        Ok((store, tracker))
    }

    pub fn persist_observation(
        &self,
        tx_hash: TransactionHash,
        entry: &GraduationJournalEntry,
    ) -> io::Result<()> {
        let tx = self.db.transaction();
        for (output_ref, _) in &entry.consumed_snek_refs {
            tx.delete(snek_pool_input_key(*output_ref))
                .map_err(rocksdb_error)?;
        }
        for (output_ref, pool_id) in &entry.produced_snek_refs {
            tx.put(snek_pool_input_key(*output_ref), token_key_value(*pool_id))
                .map_err(rocksdb_error)?;
        }
        for pool_id in &entry.graduated_splash_ids {
            tx.put(graduated_pool_key(*pool_id), []).map_err(rocksdb_error)?;
        }
        tx.put(journal_key(tx_hash), encode_journal_entry(entry))
            .map_err(rocksdb_error)?;
        tx.commit().map_err(rocksdb_error)
    }

    pub fn persist_rollback(
        &self,
        tx_hash: TransactionHash,
        entry: &GraduationJournalEntry,
    ) -> io::Result<()> {
        let tx = self.db.transaction();
        for pool_id in &entry.graduated_splash_ids {
            tx.delete(graduated_pool_key(*pool_id)).map_err(rocksdb_error)?;
        }
        for (output_ref, _) in &entry.produced_snek_refs {
            tx.delete(snek_pool_input_key(*output_ref))
                .map_err(rocksdb_error)?;
        }
        for (output_ref, pool_id) in &entry.consumed_snek_refs {
            tx.put(snek_pool_input_key(*output_ref), token_key_value(*pool_id))
                .map_err(rocksdb_error)?;
        }
        tx.delete(journal_key(tx_hash)).map_err(rocksdb_error)?;
        tx.commit().map_err(rocksdb_error)
    }
}

fn graduated_pool_key(pool_id: Token) -> Vec<u8> {
    prefixed_key(GRADUATED_POOL_KEY_PREFIX, token_key_value(pool_id))
}

fn snek_pool_input_key(output_ref: OutputRef) -> Vec<u8> {
    prefixed_key(SNEK_POOL_INPUT_KEY_PREFIX, output_ref.to_string().into_bytes())
}

fn journal_key(tx_hash: TransactionHash) -> Vec<u8> {
    prefixed_key(JOURNAL_KEY_PREFIX, tx_hash.to_hex().into_bytes())
}

fn prefixed_key(prefix: &[u8], value: Vec<u8>) -> Vec<u8> {
    let mut key = Vec::with_capacity(prefix.len() + value.len());
    key.extend_from_slice(prefix);
    key.extend_from_slice(&value);
    key
}

fn parse_token(raw: &[u8]) -> io::Result<Token> {
    let raw = str::from_utf8(raw).map_err(invalid_graduation_state)?;
    Token::try_from_string(raw).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("invalid token in graduation state: {raw}"),
        )
    })
}

fn parse_tx_hash(raw: &[u8]) -> io::Result<TransactionHash> {
    let raw = str::from_utf8(raw).map_err(invalid_graduation_state)?;
    TransactionHash::from_hex(raw).map_err(invalid_graduation_state)
}

fn token_key_value(token: Token) -> Vec<u8> {
    format!("{}.{}", token.0.to_hex(), hex::encode(token.1.as_bytes())).into_bytes()
}

fn parse_output_ref(raw: &[u8]) -> io::Result<OutputRef> {
    let raw = str::from_utf8(raw).map_err(invalid_graduation_state)?;
    let Some((raw_tx_hash, raw_index)) = raw.split_once('#') else {
        return Err(invalid_graduation_state("invalid output ref"));
    };
    let tx_hash = TransactionHash::from_hex(raw_tx_hash).map_err(invalid_graduation_state)?;
    let index = raw_index.parse::<u64>().map_err(invalid_graduation_state)?;
    Ok(OutputRef::new(tx_hash, index))
}

fn encode_journal_entry(entry: &GraduationJournalEntry) -> Vec<u8> {
    let mut out = String::new();
    for (output_ref, pool_id) in &entry.consumed_snek_refs {
        out.push_str("C ");
        out.push_str(&output_ref.to_string());
        out.push(' ');
        out.push_str(&String::from_utf8(token_key_value(*pool_id)).expect("token key is ASCII"));
        out.push('\n');
    }
    for (output_ref, pool_id) in &entry.produced_snek_refs {
        out.push_str("P ");
        out.push_str(&output_ref.to_string());
        out.push(' ');
        out.push_str(&String::from_utf8(token_key_value(*pool_id)).expect("token key is ASCII"));
        out.push('\n');
    }
    for pool_id in &entry.graduated_splash_ids {
        out.push_str("G ");
        out.push_str(&String::from_utf8(token_key_value(*pool_id)).expect("token key is ASCII"));
        out.push('\n');
    }
    out.into_bytes()
}

fn parse_journal_entry(raw: &[u8]) -> io::Result<GraduationJournalEntry> {
    let raw = str::from_utf8(raw).map_err(invalid_graduation_state)?;
    let mut consumed_snek_refs = Vec::new();
    let mut produced_snek_refs = Vec::new();
    let mut graduated_splash_ids = Vec::new();
    for line in raw.lines() {
        let mut parts = line.split(' ');
        match parts.next() {
            Some("C") => {
                let output_ref = parts
                    .next()
                    .ok_or_else(|| invalid_graduation_state("missing consumed output ref"))
                    .and_then(|raw| parse_output_ref(raw.as_bytes()))?;
                let pool_id = parts
                    .next()
                    .ok_or_else(|| invalid_graduation_state("missing consumed pool id"))
                    .and_then(|raw| parse_token(raw.as_bytes()))?;
                consumed_snek_refs.push((output_ref, pool_id));
            }
            Some("P") => {
                let output_ref = parts
                    .next()
                    .ok_or_else(|| invalid_graduation_state("missing produced output ref"))
                    .and_then(|raw| parse_output_ref(raw.as_bytes()))?;
                let pool_id = parts
                    .next()
                    .ok_or_else(|| invalid_graduation_state("missing produced pool id"))
                    .and_then(|raw| parse_token(raw.as_bytes()))?;
                produced_snek_refs.push((output_ref, pool_id));
            }
            Some("G") => {
                let pool_id = parts
                    .next()
                    .ok_or_else(|| invalid_graduation_state("missing graduated pool id"))
                    .and_then(|raw| parse_token(raw.as_bytes()))?;
                graduated_splash_ids.push(pool_id);
            }
            _ => return Err(invalid_graduation_state("invalid graduation journal record")),
        }
        if parts.next().is_some() {
            return Err(invalid_graduation_state("invalid graduation journal record"));
        }
    }
    Ok(GraduationJournalEntry {
        consumed_snek_refs,
        produced_snek_refs,
        graduated_splash_ids,
    })
}

fn rocksdb_error(err: rocksdb::Error) -> io::Error {
    io::Error::new(io::ErrorKind::Other, err.to_string())
}

fn invalid_graduation_state(err: impl std::fmt::Display) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, err.to_string())
}

#[derive(Clone, Debug, Default)]
pub struct SnekPoolInputTracker {
    inner: Arc<RwLock<HashMap<OutputRef, Token>>>,
}

impl SnekPoolInputTracker {
    pub fn contains(&self, output_ref: OutputRef) -> Option<Token> {
        self.inner
            .read()
            .expect("snek pool tracker poisoned")
            .get(&output_ref)
            .copied()
    }

    pub fn insert(&self, output_ref: OutputRef, pool_id: Token) {
        self.inner
            .write()
            .expect("snek pool tracker poisoned")
            .insert(output_ref, pool_id);
    }

    pub fn remove(&self, output_ref: OutputRef) {
        self.inner
            .write()
            .expect("snek pool tracker poisoned")
            .remove(&output_ref);
    }

    pub fn snapshot(&self) -> Vec<(OutputRef, Token)> {
        self.inner
            .read()
            .expect("snek pool tracker poisoned")
            .iter()
            .map(|(output_ref, pool_id)| (*output_ref, *pool_id))
            .collect()
    }

    pub fn rollback_tx(&self, entry: GraduationJournalEntry, graduated_store: &GraduatedSplashPoolStore) {
        let mut live_refs = self.inner.write().expect("snek pool tracker poisoned");
        for pool_id in entry.graduated_splash_ids {
            graduated_store.remove(pool_id);
        }
        for (oref, _) in entry.produced_snek_refs {
            live_refs.remove(&oref);
        }
        live_refs.extend(entry.consumed_snek_refs);
    }
}

#[derive(Clone, Debug, Default)]
pub struct GraduationJournalEntry {
    pub consumed_snek_refs: Vec<(OutputRef, Token)>,
    pub produced_snek_refs: Vec<(OutputRef, Token)>,
    pub graduated_splash_ids: Vec<Token>,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct SnekQuadraticPoolIdentity {
    pub pool_id: Token,
}

#[derive(Copy, Clone, Debug, Default, Eq, PartialEq, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SnekPoolScriptHashes {
    #[serde(rename = "quadraticPoolV1ScriptHash")]
    pub quadratic_pool_v1_script_hash: Option<ScriptHash>,
    #[serde(rename = "quadraticPoolV1T2TScriptHash")]
    pub quadratic_pool_v1_t2t_script_hash: Option<ScriptHash>,
}

impl SnekPoolScriptHashes {
    pub fn is_configured(&self) -> bool {
        self.quadratic_pool_v1_script_hash.is_some() || self.quadratic_pool_v1_t2t_script_hash.is_some()
    }

    fn pool_version(&self, repr: &TransactionOutput) -> Option<SnekQuadraticPoolVersion> {
        let hash = repr.address().payment_cred().and_then(|cred| match cred {
            StakeCredential::Script { hash, .. } => Some(*hash),
            StakeCredential::PubKey { .. } => None,
        })?;
        if self
            .quadratic_pool_v1_script_hash
            .is_some_and(|expected| expected == hash)
        {
            Some(SnekQuadraticPoolVersion::V1)
        } else if self
            .quadratic_pool_v1_t2t_script_hash
            .is_some_and(|expected| expected == hash)
        {
            Some(SnekQuadraticPoolVersion::V1T2T)
        } else {
            None
        }
    }
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
enum SnekQuadraticPoolVersion {
    V1,
    V1T2T,
}

impl SnekQuadraticPoolIdentity {
    pub fn try_from_ledger(repr: &TransactionOutput, script_hashes: SnekPoolScriptHashes) -> Option<Self> {
        let pool_ver = script_hashes.pool_version(repr)?;
        let pd = repr.datum().clone()?.into_pd()?;
        let pool_id = match pool_ver {
            SnekQuadraticPoolVersion::V1 => {
                let conf = QuadraticPoolConfig::try_from_pd(pd)?;
                PoolId::try_from(conf.pool_nft).ok()?
            }
            SnekQuadraticPoolVersion::V1T2T => {
                let conf = QuadraticPoolT2TConfig::try_from_pd(pd)?;
                PoolId::try_from(conf.pool_nft).ok()?
            }
        };
        Some(Self {
            pool_id: pool_id.into(),
        })
    }
}

#[cfg(test)]
mod tests {
    use cml_chain::transaction::TransactionOutput;
    use cml_core::serialization::Deserialize;
    use cml_crypto::{ScriptHash, TransactionHash};
    use spectrum_cardano_lib::{OutputRef, Token};

    use crate::graduation::{
        GraduatedSplashPoolStore, GraduationJournalEntry, GraduationStateRocksDb, SnekPoolInputTracker,
        SnekPoolScriptHashes, SnekQuadraticPoolIdentity,
    };

    #[test]
    fn parses_snek_quadratic_pool_identity_from_ledger_output() {
        let bearer = TransactionOutput::from_cbor_bytes(&hex::decode(SNEK_POOL_UTXO).unwrap()).unwrap();

        let identity = SnekQuadraticPoolIdentity::try_from_ledger(
            &bearer,
            SnekPoolScriptHashes {
                quadratic_pool_v1_script_hash: Some(ScriptHash::from_hex(SNEK_POOL_SCRIPT_HASH).unwrap()),
                quadratic_pool_v1_t2t_script_hash: None,
            },
        )
        .unwrap();

        assert_eq!(
            identity.pool_id,
            Token::from_string_unsafe("9d8f27a66cfffebe2a4a19157b6845a051dd2f627f11bfafed584d51.6e6674")
        );
    }

    #[test]
    fn rejects_quadratic_datum_at_unconfigured_script_hash() {
        let bearer = TransactionOutput::from_cbor_bytes(&hex::decode(SNEK_POOL_UTXO).unwrap()).unwrap();

        let identity = SnekQuadraticPoolIdentity::try_from_ledger(
            &bearer,
            SnekPoolScriptHashes {
                quadratic_pool_v1_script_hash: None,
                quadratic_pool_v1_t2t_script_hash: Some(ScriptHash::from([0u8; 28])),
            },
        );

        assert!(identity.is_none());
    }

    #[test]
    fn deserializes_snek_pool_script_hash_config() {
        let config: SnekPoolScriptHashes = serde_json::from_str(&format!(
            r#"{{
                "quadraticPoolV1ScriptHash": "{SNEK_POOL_SCRIPT_HASH}",
                "quadraticPoolV1T2TScriptHash": null
            }}"#
        ))
        .unwrap();

        assert_eq!(
            config.quadratic_pool_v1_script_hash,
            Some(ScriptHash::from_hex(SNEK_POOL_SCRIPT_HASH).unwrap())
        );
        assert_eq!(config.quadratic_pool_v1_t2t_script_hash, None);
        assert!(config.is_configured());
    }

    #[test]
    fn graduation_observation_marks_splash_pool_and_rolls_back() {
        let store = GraduatedSplashPoolStore::default();
        let tracker = SnekPoolInputTracker::default();
        let snek_ref = OutputRef::new(TransactionHash::from_hex(&"11".repeat(32)).unwrap(), 0);
        let produced_snek_ref = OutputRef::new(TransactionHash::from_hex(&"22".repeat(32)).unwrap(), 1);
        let tx_hash = TransactionHash::from_hex(&"33".repeat(32)).unwrap();
        let snek_pool_id =
            Token::from_string_unsafe("9d8f27a66cfffebe2a4a19157b6845a051dd2f627f11bfafed584d51.6e6674");
        let splash_pool_id = Token::from_string_unsafe(
            "9d8f27a66cfffebe2a4a19157b6845a051dd2f627f11bfafed584d51.73706c617368",
        );
        tracker.insert(snek_ref, snek_pool_id);

        store.apply_observation(
            tx_hash,
            &tracker,
            vec![(snek_ref, snek_pool_id)],
            vec![(produced_snek_ref, snek_pool_id)],
            vec![splash_pool_id],
        );

        assert!(store.contains(splash_pool_id));
        assert_eq!(tracker.contains(snek_ref), None);
        assert_eq!(tracker.contains(produced_snek_ref), Some(snek_pool_id));

        store.rollback_tx(tx_hash, &tracker);

        assert!(!store.contains(splash_pool_id));
        assert_eq!(tracker.contains(snek_ref), Some(snek_pool_id));
        assert_eq!(tracker.contains(produced_snek_ref), None);
    }

    #[test]
    fn graduation_observation_tracks_snek_only_update_and_rolls_back() {
        let store = GraduatedSplashPoolStore::default();
        let tracker = SnekPoolInputTracker::default();
        let consumed_snek_ref = OutputRef::new(TransactionHash::from_hex(&"44".repeat(32)).unwrap(), 0);
        let produced_snek_ref = OutputRef::new(TransactionHash::from_hex(&"55".repeat(32)).unwrap(), 1);
        let tx_hash = TransactionHash::from_hex(&"66".repeat(32)).unwrap();
        let snek_pool_id =
            Token::from_string_unsafe("9d8f27a66cfffebe2a4a19157b6845a051dd2f627f11bfafed584d51.6e6674");
        tracker.insert(consumed_snek_ref, snek_pool_id);

        store.apply_observation(
            tx_hash,
            &tracker,
            vec![(consumed_snek_ref, snek_pool_id)],
            vec![(produced_snek_ref, snek_pool_id)],
            vec![],
        );

        assert_eq!(tracker.contains(consumed_snek_ref), None);
        assert_eq!(tracker.contains(produced_snek_ref), Some(snek_pool_id));

        store.rollback_tx(tx_hash, &tracker);

        assert_eq!(tracker.contains(consumed_snek_ref), Some(snek_pool_id));
        assert_eq!(tracker.contains(produced_snek_ref), None);
    }

    #[test]
    fn graduated_pool_fee_rejects_one_hundred_percent() {
        assert!(crate::graduation::GraduatedPoolFeeConfig::try_enabled(100).is_err());
    }

    #[test]
    fn graduated_pool_fee_handles_large_amounts_without_overflow() {
        let config = crate::graduation::GraduatedPoolFeeConfig::try_enabled(99).unwrap();

        assert_eq!(config.fee(u64::MAX), 18262276632972456098);
    }

    #[test]
    fn graduation_state_rocksdb_roundtrip_restores_store_and_tracker() {
        let splash_pool_id = Token::from_string_unsafe(
            "9d8f27a66cfffebe2a4a19157b6845a051dd2f627f11bfafed584d51.73706c617368",
        );
        let snek_pool_id =
            Token::from_string_unsafe("9d8f27a66cfffebe2a4a19157b6845a051dd2f627f11bfafed584d51.6e6674");
        let snek_ref = OutputRef::new(TransactionHash::from_hex(&"77".repeat(32)).unwrap(), 0);
        let tx_hash = TransactionHash::from_hex(&"88".repeat(32)).unwrap();
        let path = temp_graduation_db_path("roundtrip");

        {
            let state = GraduationStateRocksDb::open(&path).unwrap();
            state
                .persist_observation(
                    tx_hash,
                    &GraduationJournalEntry {
                        consumed_snek_refs: vec![],
                        produced_snek_refs: vec![(snek_ref, snek_pool_id)],
                        graduated_splash_ids: vec![splash_pool_id],
                    },
                )
                .unwrap();
        }
        let state = GraduationStateRocksDb::open(&path).unwrap();
        let (loaded_store, loaded_tracker) = state.load_state().unwrap();

        assert!(loaded_store.contains(splash_pool_id));
        assert_eq!(loaded_tracker.contains(snek_ref), Some(snek_pool_id));
    }

    #[test]
    fn empty_graduation_state_rocksdb_loads_empty_state() {
        let path = temp_graduation_db_path("empty");
        let state = GraduationStateRocksDb::open(&path).unwrap();

        let (store, tracker) = state.load_state().unwrap();

        assert!(store.snapshot().is_empty());
        assert!(tracker.snapshot().is_empty());
    }

    #[test]
    fn graduation_state_rocksdb_rollback_persists_inverse_delta() {
        let consumed_snek_ref = OutputRef::new(TransactionHash::from_hex(&"99".repeat(32)).unwrap(), 0);
        let produced_snek_ref = OutputRef::new(TransactionHash::from_hex(&"aa".repeat(32)).unwrap(), 1);
        let splash_pool_id = Token::from_string_unsafe(
            "9d8f27a66cfffebe2a4a19157b6845a051dd2f627f11bfafed584d51.73706c617368",
        );
        let snek_pool_id =
            Token::from_string_unsafe("9d8f27a66cfffebe2a4a19157b6845a051dd2f627f11bfafed584d51.6e6674");
        let path = temp_graduation_db_path("rollback");
        let tx_hash = TransactionHash::from_hex(&"bb".repeat(32)).unwrap();
        let entry = GraduationJournalEntry {
            consumed_snek_refs: vec![(consumed_snek_ref, snek_pool_id)],
            produced_snek_refs: vec![(produced_snek_ref, snek_pool_id)],
            graduated_splash_ids: vec![splash_pool_id],
        };

        {
            let state = GraduationStateRocksDb::open(&path).unwrap();
            state.persist_observation(tx_hash, &entry).unwrap();
            state.persist_rollback(tx_hash, &entry).unwrap();
        }
        let state = GraduationStateRocksDb::open(&path).unwrap();
        let (store, tracker) = state.load_state().unwrap();

        assert!(!store.contains(splash_pool_id));
        assert_eq!(tracker.contains(consumed_snek_ref), Some(snek_pool_id));
        assert_eq!(tracker.contains(produced_snek_ref), None);
    }

    #[test]
    fn graduation_state_rocksdb_restores_journal_for_rollback_after_restart() {
        let consumed_snek_ref = OutputRef::new(TransactionHash::from_hex(&"cc".repeat(32)).unwrap(), 0);
        let splash_pool_id = Token::from_string_unsafe(
            "9d8f27a66cfffebe2a4a19157b6845a051dd2f627f11bfafed584d51.73706c617368",
        );
        let snek_pool_id =
            Token::from_string_unsafe("9d8f27a66cfffebe2a4a19157b6845a051dd2f627f11bfafed584d51.6e6674");
        let path = temp_graduation_db_path("rollback-after-restart");
        let tx_hash = TransactionHash::from_hex(&"dd".repeat(32)).unwrap();
        let entry = GraduationJournalEntry {
            consumed_snek_refs: vec![(consumed_snek_ref, snek_pool_id)],
            produced_snek_refs: vec![],
            graduated_splash_ids: vec![splash_pool_id],
        };

        {
            let state = GraduationStateRocksDb::open(&path).unwrap();
            state.persist_observation(tx_hash, &entry).unwrap();
        }
        {
            let state = GraduationStateRocksDb::open(&path).unwrap();
            let (store, tracker) = state.load_state().unwrap();
            let rollback_entry = store.rollback_tx(tx_hash, &tracker).unwrap();
            state.persist_rollback(tx_hash, &rollback_entry).unwrap();
        }
        let state = GraduationStateRocksDb::open(&path).unwrap();
        let (store, tracker) = state.load_state().unwrap();

        assert!(!store.contains(splash_pool_id));
        assert_eq!(tracker.contains(consumed_snek_ref), Some(snek_pool_id));
    }

    fn temp_graduation_db_path(test_name: &str) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!("graduation-state-{test_name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        path
    }

    const SNEK_POOL_UTXO: &str = "a300581d7005fca42e405386300c71cb3d3ab80ed65e2838f20073409c0cca063101821a05f5e100a2581c1954722030c9adf89d037ebe00bc70747eb746956a8b02f755f789a9a145746f6b656e1a3b9aca00581c9d8f27a66cfffebe2a4a19157b6845a051dd2f627f11bfafed584d51a1436e667401028201d81858f3d8799fd8799f581c9d8f27a66cfffebe2a4a19157b6845a051dd2f627f11bfafed584d51436e6674ffd8799f581cf357c6f00f0496fcd01851a7a8d909a1d9d1c9d7ba9bc021ac3bc3fe4d636e74546f6b656e746f6b656effd8799f581c1954722030c9adf89d037ebe00bc70747eb746956a8b02f755f789a945746f6b656eff1b0000001efc22eee61a00393870581c15772e8f1fdcf12d59636caf42522b7d6249ccb223253eb7e9b6d5091b00000004af5c9bf9581ce67c2ed0ccbea65650a054400a22357a357f581a0b535fc06097278b581c65e55e46a039c5711fcdc508c79ef626b0b4e7be0e6fb3c4548939c0ff";
    const SNEK_POOL_SCRIPT_HASH: &str = "05fca42e405386300c71cb3d3ab80ed65e2838f20073409c0cca0631";
}
