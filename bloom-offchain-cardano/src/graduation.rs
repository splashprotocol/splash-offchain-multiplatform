use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::{Arc, RwLock};

use bounded_integer::BoundedU64;
use cml_chain::transaction::TransactionOutput;
use cml_crypto::TransactionHash;
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

impl SnekQuadraticPoolIdentity {
    pub fn try_from_ledger(repr: &TransactionOutput) -> Option<Self> {
        let pd = repr.datum().clone()?.into_pd()?;
        let pool_id = QuadraticPoolConfig::try_from_pd(pd.clone())
            .and_then(|conf| PoolId::try_from(conf.pool_nft).ok())
            .or_else(|| {
                QuadraticPoolT2TConfig::try_from_pd(pd).and_then(|conf| PoolId::try_from(conf.pool_nft).ok())
            })?;
        Some(Self {
            pool_id: pool_id.into(),
        })
    }
}
