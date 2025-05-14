use crate::tx_view::{TxView, TxViewPartiallyResolved};
use cardano_chain_sync::atomic_flow::BlockEvents;
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
use log::info;

pub async fn read_events<Out, Cx, Index>(
    mut block: BlockEvents<Either<BabbageTransaction, Transaction>>,
    context: &Cx,
    index: &Index,
    persistable_entities_hashes: &HashSet<ScriptHash>,
) -> BlockEvents<Out>
where
    Out: TryFromLedger<TxViewPartiallyResolved, Cx>,
    Index: PersistentIndex<OutputRef, TransactionOutput>,
{
    let (txs, slot) = match &mut block {
        BlockEvents::RollForward {
            events, block_slot, ..
        }
        | BlockEvents::RollBackward {
            events, block_slot, ..
        } => (events.drain(0..), block_slot),
    };

    let events = stream::iter(txs)
        .map(TxView::from)
        .then(|tx| async move {
            persist_suitable_entities(&tx, index, persistable_entities_hashes).await;
            tx
        })
        .then(|tx| TxViewPartiallyResolved::resolve(tx, index, *slot))
        .collect::<Vec<_>>()
        .await
        .into_iter()
        .filter_map(|tx| Out::try_from_ledger(&tx, &context))
        .collect();
    block.map(|_| events)
}

async fn persist_suitable_entities<Index: PersistentIndex<OutputRef, TransactionOutput>>(
    tx: &TxView,
    index: &Index,
    persistable_entities_hashes: &HashSet<ScriptHash>,
) {
    for (ix, o) in tx.outputs.iter().enumerate() {
        if test_address(o.address(), persistable_entities_hashes) {
            info!("[Persist] Persist entity {}#{}", tx.hash.to_hex(), ix);
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
