use cardano_chain_sync::atomic_flow::BlockEvents;
use cml_chain::transaction::Transaction;
use cml_crypto::TransactionHash;
use cml_multi_era::babbage::BabbageTransaction;
use either::Either;
use futures::{Sink, SinkExt};
use spectrum_cardano_lib::hash::hash_transaction_canonical;
use std::fmt::Debug;

pub(crate) async fn forward_confirmed_txs<S>(
    block: &BlockEvents<Either<BabbageTransaction, Transaction>>,
    mut channel: S,
) where
    S: Sink<(TransactionHash, u64)> + Unpin,
    S::Error: Debug,
{
    match block {
        BlockEvents::RollForward {
            events, block_num, ..
        } => {
            let txs = events.iter().map(|tx| {
                (
                    match tx {
                        Either::Left(tx) => hash_transaction_canonical(tx),
                        Either::Right(tx) => hash_transaction_canonical(tx),
                    },
                    *block_num,
                )
            });
            for tx in txs {
                channel.feed(tx).await.unwrap();
            }
            channel.flush().await.unwrap();
        }
        BlockEvents::RollBackward { .. } => {}
    }
}
