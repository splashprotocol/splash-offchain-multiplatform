use crate::onchain::event::{BufferWalletAddress, OnChainEvent};
use cardano_chain_sync::atomic_flow::BlockEvents;
use cml_chain::address::Address;
use cml_chain::certs::StakeCredential;
use cml_chain::transaction::{Transaction, TransactionOutput};
use cml_crypto::{ScriptHash, TransactionHash};
use cml_multi_era::babbage::BabbageTransaction;
use either::Either;
use futures::channel::mpsc::Sender;
use futures::{stream, SinkExt, StreamExt};
use log::info;
use spectrum_cardano_lib::{NetworkId, OutputRef};
use spectrum_offchain::domain::Has;
use spectrum_offchain::ledger::TryFromLedger;
use spectrum_offchain::persistent_index::PersistentIndex;
use spectrum_offchain_cardano::deployment::DeployedScriptInfo;
use splash_dao_offchain::deployment::ProtocolValidator;
use splash_distribution::entities::events::user::withdraw::UserWithdraw;
use splash_lp_indexer::tx_view::{TxView, TxViewPartiallyResolved};
use std::collections::HashSet;
use spectrum_cardano_lib::output::FinalizedTxOut;
use splash_dao_offchain::entities::Snapshot;
use splash_dao_offchain::protocol_config::{FarmAuthPolicy, PermManagerAuthPolicy, SplashPolicy};
use splash_dao_offchain::routines::TimedOutputRef;

// pub async fn read_events<Cx, Index>(
//     mut block: BlockEvents<Either<BabbageTransaction, Transaction>>,
//     context: &Cx,
//     index: &Index,
//     utxo_filter: &HashSet<ScriptHash>,
//     confirmed_txs: Sender<(TransactionHash, u64)>,
// ) -> BlockEvents<Snapshot<OnChainEventSnapshots, FinalizedTxOut>>
// where
//     Cx: Has<DeployedScriptInfo<{ ProtocolValidator::HarvestOrder as u8 }>>
//         + Has<HarvestLimits>
//         + Has<NetworkId>
//         + Has<PermManagerAuthPolicy>
//         + Has<FarmAuthPolicy>
//         + Has<SplashPolicy>
//         + Has<DeployedScriptInfo<{ ProtocolValidator::SmartFarm as u8 }>>
//         + Has<BufferWalletAddress>
//         + Clone,
//     Index: PersistentIndex<OutputRef, TransactionOutput>,
// {
//     let (txs, slot) = match &mut block {
//         BlockEvents::RollForward {
//             events, block_slot, ..
//         }
//         | BlockEvents::RollBackward {
//             events, block_slot, ..
//         } => (events.drain(0..), block_slot),
//     };
//
//     let events = stream::iter(txs)
//         .map(TxView::from)
//         .then(|tx| {
//             let tx_hash = tx.hash.clone();
//             let slot_u64 = slot.clone();
//             let mut sender = confirmed_txs.clone();
//             info!("Going to index utxo in async");
//             async move {
//                 sender.send((tx_hash, slot_u64)).await.unwrap();
//                 info!("In async");
//                 index_utxos(&tx, index, utxo_filter).await;
//                 info!("after async");
//                 tx
//             }
//         })
//         .then(|tx| {
//             info!("before partially resolving");
//             let res = TxViewPartiallyResolved::resolve(tx, index, *slot);
//             info!("after partially resolving");
//             res
//         })
//         .collect::<Vec<_>>()
//         .await
//         .into_iter()
//         .flat_map(|tx| {
//             info!("1");
//             let mut parsed_events: Vec<Snapshot<OnChainEventSnapshots, FinalizedTxOut>> = vec![];
//             info!("2");
//             for (idx, output) in tx.outputs.clone().into_iter().enumerate() {
//                 if let Some(event) = OnChainEventSnapshots::try_from_ledger(&tx, context) {
//                     parsed_events.push(
//                         Snapshot(
//                             event,
//                             FinalizedTxOut(
//                                 output,
//                                 OutputRef(
//                                     tx.hash,
//                                     idx as u64
//                                 )
//                             )
//                         )
//                     )
//                 }
//             }
//             info!("3");
//             parsed_events
//         })
//         .collect();
//     block.map(|_| events)
// }

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
