use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::{Arc, RwLock};

use bounded_integer::BoundedU64;
use cml_chain::certs::StakeCredential;
use cml_chain::transaction::TransactionOutput;
use cml_crypto::{ScriptHash, TransactionHash};
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
    pub relative_fee_percent: BoundedU64<0, 100>,
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
    pub fn enabled(relative_fee_percent: u64) -> Self {
        Self {
            enabled: true,
            relative_fee_percent: BoundedU64::new_saturating(relative_fee_percent),
        }
    }

    pub fn fee(&self, amount: u64) -> u64 {
        if self.enabled {
            amount * self.relative_fee_percent.get() / 100
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
    ) {
        for (output_ref, _) in &consumed_snek_refs {
            tracker.remove(*output_ref);
        }
        for (output_ref, pool_id) in &produced_snek_refs {
            tracker.insert(*output_ref, *pool_id);
        }
        self.extend(graduated_splash_ids.iter().copied());
        self.journal_applied(
            tx_hash,
            GraduationJournalEntry {
                consumed_snek_refs,
                produced_snek_refs,
                graduated_splash_ids,
            },
        );
    }

    pub fn rollback_tx(&self, tx_hash: TransactionHash, tracker: &SnekPoolInputTracker) {
        if let Some(entry) = self
            .journal
            .write()
            .expect("graduation journal poisoned")
            .remove(&tx_hash)
        {
            tracker.rollback_tx(entry, self);
        }
    }

    pub fn load_from_disk(_: &Path) -> std::io::Result<Self> {
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "graduated pool store persistence is not implemented yet",
        ))
    }

    pub fn flush_to_disk(&self, _: &Path) -> std::io::Result<()> {
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "graduated pool store persistence is not implemented yet",
        ))
    }
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
        GraduatedSplashPoolStore, SnekPoolInputTracker, SnekPoolScriptHashes, SnekQuadraticPoolIdentity,
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

    const SNEK_POOL_UTXO: &str = "a300581d7005fca42e405386300c71cb3d3ab80ed65e2838f20073409c0cca063101821a05f5e100a2581c1954722030c9adf89d037ebe00bc70747eb746956a8b02f755f789a9a145746f6b656e1a3b9aca00581c9d8f27a66cfffebe2a4a19157b6845a051dd2f627f11bfafed584d51a1436e667401028201d81858f3d8799fd8799f581c9d8f27a66cfffebe2a4a19157b6845a051dd2f627f11bfafed584d51436e6674ffd8799f581cf357c6f00f0496fcd01851a7a8d909a1d9d1c9d7ba9bc021ac3bc3fe4d636e74546f6b656e746f6b656effd8799f581c1954722030c9adf89d037ebe00bc70747eb746956a8b02f755f789a945746f6b656eff1b0000001efc22eee61a00393870581c15772e8f1fdcf12d59636caf42522b7d6249ccb223253eb7e9b6d5091b00000004af5c9bf9581ce67c2ed0ccbea65650a054400a22357a357f581a0b535fc06097278b581c65e55e46a039c5711fcdc508c79ef626b0b4e7be0e6fb3c4548939c0ff";
    const SNEK_POOL_SCRIPT_HASH: &str = "05fca42e405386300c71cb3d3ab80ed65e2838f20073409c0cca0631";
}
