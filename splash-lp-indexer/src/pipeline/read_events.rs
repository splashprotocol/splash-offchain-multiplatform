use crate::tx_view::{TxView, TxViewPartiallyResolved};
use cardano_chain_sync::data::LedgerBlockEvent;
use cml_chain::address::Address;
use cml_chain::certs::StakeCredential;
use cml_chain::transaction::{Transaction, TransactionOutput};
use cml_crypto::ScriptHash;
use cml_multi_era::babbage::BabbageTransaction;
use either::Either;
use futures::{stream, StreamExt};
use spectrum_cardano_lib::OutputRef;
use spectrum_offchain::ledger::TryFromLedger;
use spectrum_offchain::persistent_index::PersistentIndex;
use std::collections::HashSet;

pub async fn read_events<Out, Cx, Index>(
    mut block: LedgerBlockEvent<Vec<Either<BabbageTransaction, Transaction>>>,
    context: &Cx,
    index: &Index,
    utxo_filter: &HashSet<ScriptHash>,
) -> LedgerBlockEvent<Vec<Out>>
where
    Out: TryFromLedger<TxViewPartiallyResolved, Cx>,
    Index: PersistentIndex<OutputRef, TransactionOutput>,
{
    let txs = match &mut block {
        LedgerBlockEvent::RollForward(content) | LedgerBlockEvent::RollBackward(content) => {
            content.drain(0..)
        }
    };
    let events = stream::iter(txs)
        .map(TxView::from)
        .then(|tx| async move {
            index_utxos(&tx, index, utxo_filter).await;
            tx
        })
        .then(|tx| TxViewPartiallyResolved::resolve(tx, index))
        .collect::<Vec<_>>()
        .await
        .into_iter()
        .filter_map(|tx| Out::try_from_ledger(&tx, &context))
        .collect();
    block.map(|_| events)
}

async fn index_utxos<Index: PersistentIndex<OutputRef, TransactionOutput>>(
    tx: &TxView,
    index: &Index,
    utxo_filter: &HashSet<ScriptHash>,
) {
    for (ix, o) in tx.outputs.iter().enumerate() {
        if test_address(o.address(), utxo_filter) {
            let oref = OutputRef::new(tx.hash, ix as u64);
            index.insert(oref, o.clone()).await;
        }
    }
}

pub fn test_address(addr: &Address, utxo_filter: &HashSet<ScriptHash>) -> bool {
    let maybe_hash = addr.payment_cred().and_then(|c| match c {
        StakeCredential::PubKey { .. } => None,
        StakeCredential::Script { hash, .. } => Some(hash),
    });
    if let Some(this_hash) = maybe_hash {
        return utxo_filter.contains(&this_hash);
    }
    false
}
