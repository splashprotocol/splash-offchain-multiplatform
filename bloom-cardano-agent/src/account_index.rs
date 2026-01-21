use std::collections::HashMap;
use std::time::{Duration, SystemTime};

use cml_crypto::blake2b256;
use log::{debug, trace};

use bloom_offchain_cardano::orders::green::{Account, Intent, MptDelta, MptSnapshot};
use spectrum_cardano_lib::output::FinalizedTxOut;
use spectrum_cardano_lib::Token;

/// Entry for a single account in the index.
#[derive(Debug, Clone)]
pub struct AccountEntry {
    /// Current account state
    pub account: Account,
    /// Current account UTxO
    pub utxo: FinalizedTxOut,
    /// Local intent store (intent_digest -> Intent with remainder)
    /// This mirrors the on-chain MPT stored in account.store
    intents: HashMap<[u8; 32], Intent>,
    /// Cached root hash (recomputed on modification)
    cached_root: [u8; 32],
    /// Last update timestamp
    pub last_updated: SystemTime,
}

impl AccountEntry {
    /// Create a new account entry with empty intent store.
    pub fn new(account: Account, utxo: FinalizedTxOut) -> Self {
        Self {
            cached_root: account.store,
            account,
            utxo,
            intents: HashMap::new(),
            last_updated: SystemTime::now(),
        }
    }

    /// Get the current MPT root hash.
    pub fn root_hash(&self) -> [u8; 32] {
        self.cached_root
    }

    /// Create an MptSnapshot for embedding in GreenOrder.
    pub fn create_snapshot(&self, pending_intent: Option<(Intent, [u8; 32])>) -> MptSnapshot {
        match pending_intent {
            Some((intent, digest)) => MptSnapshot::with_pending(self.cached_root, intent, digest),
            None => MptSnapshot::new(self.cached_root),
        }
    }

    /// Insert a new intent (first execution partial fill).
    /// Returns the new root hash.
    pub fn insert_intent(&mut self, digest: [u8; 32], intent: Intent) -> [u8; 32] {
        self.intents.insert(digest, intent);
        self.recompute_root();
        self.last_updated = SystemTime::now();
        self.cached_root
    }

    /// Update an existing intent (continuation partial fill).
    /// Returns the new root hash.
    pub fn update_intent(
        &mut self,
        old_digest: [u8; 32],
        new_digest: [u8; 32],
        intent: Intent,
    ) -> [u8; 32] {
        self.intents.remove(&old_digest);
        self.intents.insert(new_digest, intent);
        self.recompute_root();
        self.last_updated = SystemTime::now();
        self.cached_root
    }

    /// Delete an intent (continuation complete fill).
    /// Returns the new root hash.
    pub fn delete_intent(&mut self, digest: [u8; 32]) -> [u8; 32] {
        self.intents.remove(&digest);
        self.recompute_root();
        self.last_updated = SystemTime::now();
        self.cached_root
    }

    /// Get an intent by digest.
    pub fn get_intent(&self, digest: &[u8; 32]) -> Option<&Intent> {
        self.intents.get(digest)
    }

    /// Check if there are pending intents (partial fills).
    pub fn has_pending_intents(&self) -> bool {
        !self.intents.is_empty()
    }

    /// Get all pending intents (for continuation scanner).
    pub fn pending_intents(&self) -> impl Iterator<Item = (&[u8; 32], &Intent)> {
        self.intents.iter()
    }

    /// Recompute the MPT root hash from current intents.
    /// Uses a sorted-hash merkle tree approach for determinism.
    fn recompute_root(&mut self) {
        if self.intents.is_empty() {
            self.cached_root = [0u8; 32];
            return;
        }

        // Sort digests for deterministic ordering
        let mut digests: Vec<[u8; 32]> = self.intents.keys().copied().collect();
        digests.sort();

        // Build merkle root from sorted leaf hashes
        let mut hashes: Vec<[u8; 32]> = digests;

        while hashes.len() > 1 {
            let mut next_level = Vec::with_capacity((hashes.len() + 1) / 2);
            for chunk in hashes.chunks(2) {
                let combined = match chunk {
                    [left, right] => {
                        let mut data = Vec::with_capacity(64);
                        data.extend(left);
                        data.extend(right);
                        blake2b256(&data)
                    }
                    [single] => *single,
                    _ => unreachable!(),
                };
                next_level.push(combined);
            }
            hashes = next_level;
        }

        self.cached_root = hashes[0];
    }

    /// Generate a proof for an intent (for Auth::Path).
    /// Returns proof bytes that can be verified on-chain.
    pub fn generate_proof(&self, digest: &[u8; 32]) -> Option<Vec<u8>> {
        if !self.intents.contains_key(digest) {
            return None;
        }

        // Sort digests for deterministic ordering
        let mut digests: Vec<[u8; 32]> = self.intents.keys().copied().collect();
        digests.sort();

        let index = digests.iter().position(|d| d == digest)?;

        // Build merkle proof (sibling hashes along the path to root)
        let mut proof = Vec::new();
        let mut hashes: Vec<[u8; 32]> = digests;
        let mut pos = index;

        while hashes.len() > 1 {
            let sibling_pos = if pos % 2 == 0 { pos + 1 } else { pos - 1 };

            // Add sibling to proof if it exists
            if sibling_pos < hashes.len() {
                // Direction indicator: 0 = sibling is right, 1 = sibling is left
                proof.push(if pos % 2 == 0 { 0u8 } else { 1u8 });
                proof.extend(&hashes[sibling_pos]);
            }

            // Move to next level
            let mut next_level = Vec::with_capacity((hashes.len() + 1) / 2);
            for chunk in hashes.chunks(2) {
                let combined = match chunk {
                    [left, right] => {
                        let mut data = Vec::with_capacity(64);
                        data.extend(left);
                        data.extend(right);
                        blake2b256(&data)
                    }
                    [single] => *single,
                    _ => unreachable!(),
                };
                next_level.push(combined);
            }
            hashes = next_level;
            pos /= 2;
        }

        Some(proof)
    }
}

/// In-memory index of accounts and their MPT state.
#[derive(Clone)]
pub struct AccountIndex {
    /// Mapping from account identifier (NFT Token) to account entry
    accounts: HashMap<Token, AccountEntry>,
    /// Eviction delay for stale entries
    eviction_delay: Duration,
}

impl AccountIndex {
    /// Create a new empty account index.
    pub fn new(eviction_delay: Duration) -> Self {
        Self {
            accounts: HashMap::new(),
            eviction_delay,
        }
    }

    /// Insert or update an account.
    pub fn put(&mut self, account_id: Token, account: Account, utxo: FinalizedTxOut) {
        trace!(
            "AccountIndex::put account_id={}, store={:?}",
            account_id,
            hex::encode(account.store)
        );

        match self.accounts.get_mut(&account_id) {
            Some(entry) => {
                // Update existing entry, preserve intents
                entry.account = account;
                entry.utxo = utxo;
                entry.last_updated = SystemTime::now();
            }
            None => {
                self.accounts
                    .insert(account_id, AccountEntry::new(account, utxo));
            }
        }
    }

    /// Get an account entry.
    pub fn get(&self, account_id: &Token) -> Option<&AccountEntry> {
        self.accounts.get(account_id)
    }

    /// Get a mutable account entry.
    pub fn get_mut(&mut self, account_id: &Token) -> Option<&mut AccountEntry> {
        self.accounts.get_mut(account_id)
    }

    /// Check if an account exists.
    pub fn contains(&self, account_id: &Token) -> bool {
        self.accounts.contains_key(account_id)
    }

    /// Apply an MptDelta to update the index after successful execution.
    pub fn apply_delta(&mut self, delta: MptDelta) {
        match delta {
            MptDelta::Insert {
                account_id,
                intent_digest,
                updated_intent,
            } => {
                if let Some(entry) = self.accounts.get_mut(&account_id) {
                    let new_root = entry.insert_intent(intent_digest, updated_intent);
                    entry.account.store = new_root;
                    debug!(
                        "AccountIndex::apply_delta Insert account={}, digest={}, new_root={}",
                        account_id,
                        hex::encode(intent_digest),
                        hex::encode(new_root)
                    );
                }
            }
            MptDelta::Update {
                account_id,
                old_digest,
                new_digest,
                updated_intent,
            } => {
                if let Some(entry) = self.accounts.get_mut(&account_id) {
                    let new_root = entry.update_intent(old_digest, new_digest, updated_intent);
                    entry.account.store = new_root;
                    debug!(
                        "AccountIndex::apply_delta Update account={}, old={}, new={}, new_root={}",
                        account_id,
                        hex::encode(old_digest),
                        hex::encode(new_digest),
                        hex::encode(new_root)
                    );
                }
            }
            MptDelta::Delete {
                account_id,
                intent_digest,
            } => {
                if let Some(entry) = self.accounts.get_mut(&account_id) {
                    let new_root = entry.delete_intent(intent_digest);
                    entry.account.store = new_root;
                    debug!(
                        "AccountIndex::apply_delta Delete account={}, digest={}, new_root={}",
                        account_id,
                        hex::encode(intent_digest),
                        hex::encode(new_root)
                    );
                }
            }
            MptDelta::None => {
                trace!("AccountIndex::apply_delta None (no change)");
            }
        }
    }

    /// Update the UTxO reference for an account after a transaction.
    pub fn update_utxo(&mut self, account_id: &Token, utxo: FinalizedTxOut) {
        if let Some(entry) = self.accounts.get_mut(account_id) {
            entry.utxo = utxo;
            entry.last_updated = SystemTime::now();
        }
    }

    /// Get all accounts with pending intents (for continuation scanner).
    pub fn accounts_with_pending_intents(&self) -> Vec<(Token, &AccountEntry)> {
        self.accounts
            .iter()
            .filter(|(_, entry)| entry.has_pending_intents())
            .map(|(id, entry)| (*id, entry))
            .collect()
    }

    /// Remove stale entries that haven't been updated recently.
    pub fn run_eviction(&mut self) {
        let now = SystemTime::now();
        let threshold = now - self.eviction_delay;

        self.accounts.retain(|_, entry| {
            // Keep entries that have pending intents regardless of staleness
            entry.has_pending_intents() || entry.last_updated > threshold
        });
    }

    /// Get the number of accounts in the index.
    pub fn len(&self) -> usize {
        self.accounts.len()
    }

    /// Check if the index is empty.
    pub fn is_empty(&self) -> bool {
        self.accounts.is_empty()
    }
}

/// Wrapper with tracing for debugging.
pub struct AccountIndexTracing {
    inner: AccountIndex,
    tag: String,
}

impl AccountIndexTracing {
    pub fn attach(index: AccountIndex, tag: &str) -> Self {
        Self {
            inner: index,
            tag: String::from(tag),
        }
    }

    pub fn put(&mut self, account_id: Token, account: Account, utxo: FinalizedTxOut) {
        trace!("[{}] AccountIndex::put({})", self.tag, account_id);
        self.inner.put(account_id, account, utxo);
    }

    pub fn get(&self, account_id: &Token) -> Option<&AccountEntry> {
        let result = self.inner.get(account_id);
        trace!(
            "[{}] AccountIndex::get({}) -> {}",
            self.tag,
            account_id,
            result.is_some()
        );
        result
    }

    pub fn get_mut(&mut self, account_id: &Token) -> Option<&mut AccountEntry> {
        trace!("[{}] AccountIndex::get_mut({})", self.tag, account_id);
        self.inner.get_mut(account_id)
    }

    pub fn apply_delta(&mut self, delta: MptDelta) {
        trace!("[{}] AccountIndex::apply_delta({:?})", self.tag, delta);
        self.inner.apply_delta(delta);
    }

    pub fn accounts_with_pending_intents(&self) -> Vec<(Token, &AccountEntry)> {
        let result = self.inner.accounts_with_pending_intents();
        trace!(
            "[{}] AccountIndex::accounts_with_pending_intents() -> {} accounts",
            self.tag,
            result.len()
        );
        result
    }

    pub fn run_eviction(&mut self) {
        trace!("[{}] AccountIndex::run_eviction()", self.tag);
        self.inner.run_eviction();
    }

    pub fn inner(&self) -> &AccountIndex {
        &self.inner
    }

    pub fn inner_mut(&mut self) -> &mut AccountIndex {
        &mut self.inner
    }
}

