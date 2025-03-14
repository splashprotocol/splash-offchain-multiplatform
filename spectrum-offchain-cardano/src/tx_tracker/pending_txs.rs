use log::trace;
use std::cmp::Reverse;
use std::collections::btree_map::Entry;
use std::collections::{BTreeMap, HashMap};
use std::fmt::Display;
use std::hash::Hash;
use std::time::Instant;

pub(crate) struct PendingTxs<TxHash, Tx> {
    max_confirmation_delay_blocks: u64,
    current_block: u64,
    index: HashMap<TxHash, u64>,
    queue: BTreeMap<u64, HashMap<TxHash, (Tx, Instant)>>,
}

impl<TxHash, Tx> PendingTxs<TxHash, Tx> {
    pub fn new(max_confirmation_delay_blocks: u64) -> Self {
        Self {
            max_confirmation_delay_blocks,
            current_block: 0,
            index: Default::default(),
            queue: Default::default(),
        }
    }

    pub fn append(&mut self, tx: TxHash, trs: Tx)
    where
        TxHash: Copy + Eq + Hash,
    {
        let should_confirm_until = self.current_block + self.max_confirmation_delay_blocks;
        self.index.insert(tx, should_confirm_until);
        let now = Instant::now();
        match self.queue.entry(should_confirm_until) {
            Entry::Vacant(entry) => {
                let mut txs = HashMap::new();
                txs.insert(tx, (trs, now));
                entry.insert(txs);
            }
            Entry::Occupied(mut entry) => {
                entry.get_mut().insert(tx, (trs, now));
            }
        }
    }

    pub fn confirm_tx(&mut self, tx: TxHash)
    where
        TxHash: Copy + Eq + Hash + Display,
    {
        if let Some(key) = self.index.remove(&tx) {
            if let Some(txs) = self.queue.get_mut(&key) {
                let removed = txs.remove(&tx);
                trace!("[PendingTxs]: removed confirmed TX {}: {}", tx, removed.is_some());
            }
        }
    }

    /// Returns unsuccessful TXs in reverse-submission order so that chained-TXs are properly
    /// ordered for correct rollback.
    pub fn try_advance(&mut self, new_block: u64) -> Option<Vec<Tx>>
    where
        TxHash: Copy + Eq + Hash + Display,
    {
        if self.current_block < new_block {
            self.current_block = new_block;
            let mut failed_txs_with_timestamp = BTreeMap::new();
            loop {
                if let Some(entry) = self.queue.first_entry() {
                    if *entry.key() <= new_block {
                        let txs = entry.remove();
                        for (hash, (tx, instant)) in txs {
                            trace!("Tx {} failed", hash);
                            self.index.remove(&hash);
                            failed_txs_with_timestamp.insert(Reverse(instant), tx);
                        }
                        continue;
                    }
                }
                break;
            }

            // Reverse chronological order
            let failed_txs = failed_txs_with_timestamp.into_iter().map(|x| x.1).collect();
            trace!(
                "[TxTracker] Queue size: {}, pending transactions: {}",
                self.queue.len(),
                self.index.len()
            );
            return Some(failed_txs);
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use crate::tx_tracker::pending_txs::PendingTxs;

    #[test]
    fn track_transaction_successful() {
        let mut txs = PendingTxs::new(10);
        let tx1 = (0, vec![1, 2, 3]);
        let tx2 = (1, vec![4, 5, 6]);
        txs.append(tx1.0, tx1.1.clone());
        txs.append(tx2.0, tx2.1.clone());
        txs.confirm_tx(0);
        let unsuccessful_txs = txs.try_advance(128);
        assert_eq!(unsuccessful_txs, Some(vec![tx2.1]));
        assert!(txs.index.is_empty());
    }

    #[test]
    fn track_transaction_unsuccessful() {
        let mut txs = PendingTxs::new(10);
        let tx1 = (0, vec![1, 2, 3]);
        let tx2 = (1, vec![4, 5, 6]);
        txs.append(tx1.0, tx1.1.clone());
        txs.append(tx2.0, tx2.1.clone());
        let unsuccessful_txs = txs.try_advance(128);
        assert_eq!(unsuccessful_txs.unwrap(), vec![tx2.1, tx1.1]);
        assert!(txs.index.is_empty());
    }

    #[test]
    fn track_transaction_advance_gradually() {
        let mut txs = PendingTxs::new(10);
        let tx1 = (0, vec![1, 2, 3]);
        let tx2 = (1, vec![4, 5, 6]);
        let tx3 = (2, vec![7, 8, 9]);
        let _ = txs.try_advance(120);
        txs.append(tx1.0, tx1.1.clone());
        txs.append(tx2.0, tx2.1.clone());
        let unsuccessful_txs = txs.try_advance(128);
        assert_eq!(unsuccessful_txs, Some(vec![]));
        assert!(!txs.index.is_empty());
        txs.append(tx3.0, tx3.1.clone());
        let unsuccessful_txs = txs.try_advance(130);
        assert_eq!(unsuccessful_txs.unwrap(), vec![tx2.1, tx1.1]);
        let unsuccessful_txs = txs.try_advance(138);
        assert_eq!(unsuccessful_txs, Some(vec![tx3.1]));
    }
}
