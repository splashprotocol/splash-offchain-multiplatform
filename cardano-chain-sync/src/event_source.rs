use std::collections::HashSet;

use cml_chain::block::Block;
use cml_chain::transaction::Transaction;
use futures::stream::StreamExt;
use futures::{stream, Stream};

use crate::model::{ChainUpgrade, LedgerTxEvent};

pub fn event_source_ledger<S>(upstream: S) -> impl Stream<Item = LedgerTxEvent>
where
    S: Stream<Item = ChainUpgrade>,
{
    upstream.flat_map(|u| stream::iter(process_upgrade(u)))
}

fn process_upgrade(upgr: ChainUpgrade) -> Vec<LedgerTxEvent> {
    match upgr {
        ChainUpgrade::RollForward(Block {
            transaction_bodies,
            transaction_witness_sets,
            mut auxiliary_data_set,
            invalid_transactions,
            ..
        }) => {
            let invalid_indices: HashSet<u16> = HashSet::from_iter(invalid_transactions);
            transaction_bodies
                .into_iter()
                .zip(transaction_witness_sets)
                .enumerate()
                .map(|(ix, (tb, tw))| {
                    let tx_ix = &(ix as u16);
                    LedgerTxEvent::TxApplied(Transaction {
                        body: tb,
                        witness_set: tw,
                        is_valid: invalid_indices.contains(tx_ix),
                        auxiliary_data: auxiliary_data_set.remove(tx_ix),
                        encodings: None,
                    })
                })
                .collect()
        }
        ChainUpgrade::RollBackward(_) => Vec::new(),
    }
}
