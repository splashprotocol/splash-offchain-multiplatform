mod pending_txs;
mod tx_store;

use crate::tx_tracker::pending_txs::PendingTxs;
use async_trait::async_trait;
use futures::channel::mpsc;
use futures::stream::FusedStream;
use futures::{select, FutureExt, Sink, SinkExt, StreamExt};
use log::trace;
use spectrum_offchain::data::circular_filter::CircularFilter;
use spectrum_offchain::sink::BatchSinkExt;
use std::fmt::{Debug, Display};
use std::hash::Hash;
use std::marker::PhantomData;
use std::sync::{Arc, Mutex};

#[async_trait]
pub trait TxTracker<TxHash, Tx> {
    async fn track(&mut self, tx_hash: TxHash, tx: Tx);
}

pub struct NoopTxTracker<TxHash, Tx>(PhantomData<(TxHash, Tx)>);
impl<TxHash, Tx> NoopTxTracker<TxHash, Tx> {
    pub fn new() -> Self {
        Self(PhantomData)
    }
}

#[async_trait]
impl<TxHash: Send, Tx: Send> TxTracker<TxHash, Tx> for NoopTxTracker<TxHash, Tx> {
    async fn track(&mut self, _: TxHash, _: Tx) {}
}

#[derive(Clone)]
pub struct TxTrackerChannel<TxHash, Tx>(pub mpsc::Sender<(TxHash, Tx)>);

#[async_trait]
impl<TxHash: Send, Tx: Send> TxTracker<TxHash, Tx> for TxTrackerChannel<TxHash, Tx> {
    async fn track(&mut self, tx_hash: TxHash, tx: Tx) {
        self.0.send((tx_hash, tx)).await.unwrap();
    }
}

pub struct TxTrackerAgent<TxHash, Tx, UnconfirmedIn, ConfirmedIn, FailedOut> {
    txs_to_track: UnconfirmedIn,
    failed_txs: FailedOut,
    ledger_stream: ConfirmedIn,
    /// Hashes of recently confirmed transactions.
    recent_txs: Arc<Mutex<CircularFilter<256, TxHash>>>,
    pending_txs: Arc<Mutex<PendingTxs<TxHash, Tx>>>,
}

pub fn new_tx_tracker_bundle<TxHash, Tx, ConfirmedIn, FailedOut>(
    ledger_stream: ConfirmedIn,
    failed_txs: FailedOut,
    channel_size: usize,
    max_confirmation_delay_blocks: u64,
) -> (
    TxTrackerAgent<TxHash, Tx, mpsc::Receiver<(TxHash, Tx)>, ConfirmedIn, FailedOut>,
    TxTrackerChannel<TxHash, Tx>,
) {
    let (in_snd, in_recv) = mpsc::channel(channel_size);
    let channel_in = TxTrackerChannel(in_snd);
    let agent = TxTrackerAgent {
        txs_to_track: in_recv,
        failed_txs,
        ledger_stream,
        recent_txs: Arc::new(Mutex::new(CircularFilter::new())),
        pending_txs: Arc::new(Mutex::new(PendingTxs::new(max_confirmation_delay_blocks))),
    };
    (agent, channel_in)
}

impl<TxHash, Tx, UnconfirmedIn, ConfirmedIn, FailedOut>
    TxTrackerAgent<TxHash, Tx, UnconfirmedIn, ConfirmedIn, FailedOut>
{
    pub async fn run(self)
    where
        UnconfirmedIn: FusedStream<Item = (TxHash, Tx)> + Unpin,
        ConfirmedIn: FusedStream<Item = (TxHash, u64)> + Unpin,
        FailedOut: Sink<Tx> + Unpin,
        FailedOut::Error: Debug,
        TxHash: Copy + Eq + Hash + Display,
    {
        select! {
            _ = Self::process_incoming_txs(
                self.txs_to_track,
                Arc::clone(&self.recent_txs),
                Arc::clone(&self.pending_txs)
            ).fuse() => (),
            _ = Self::process_confirmed_txs(
                self.ledger_stream,
                self.failed_txs,
                Arc::clone(&self.recent_txs),
                Arc::clone(&self.pending_txs)
            ).fuse() => (),
        }
    }

    async fn process_incoming_txs(
        mut txs_to_track: UnconfirmedIn,
        recent_txs: Arc<Mutex<CircularFilter<256, TxHash>>>,
        pending_txs: Arc<Mutex<PendingTxs<TxHash, Tx>>>,
    ) where
        UnconfirmedIn: FusedStream<Item = (TxHash, Tx)> + Unpin,
        TxHash: Copy + Eq + Hash + Display,
    {
        loop {
            let (tx, trs) = txs_to_track.select_next_some().await;
            let already_confirmed = recent_txs.lock().unwrap().contains(&tx);
            trace!("Tracking tx: {}, already_confirmed: {}", tx, already_confirmed);
            if !already_confirmed {
                let mut pending_txs = pending_txs.lock().unwrap();
                pending_txs.append(tx, trs);
            }
        }
    }

    async fn process_confirmed_txs(
        mut ledger_stream: ConfirmedIn,
        mut failed_txs: FailedOut,
        recent_txs: Arc<Mutex<CircularFilter<256, TxHash>>>,
        pending_txs: Arc<Mutex<PendingTxs<TxHash, Tx>>>,
    ) where
        ConfirmedIn: FusedStream<Item = (TxHash, u64)> + Unpin,
        FailedOut: Sink<Tx> + Unpin,
        FailedOut::Error: Debug,
        TxHash: Copy + Eq + Hash + Display,
    {
        loop {
            let (tx, block) = ledger_stream.select_next_some().await;
            recent_txs.lock().unwrap().add(tx);
            let advance_result = {
                let mut pending_txs = pending_txs.lock().unwrap();
                pending_txs.confirm_tx(tx);
                pending_txs.try_advance(block)
            };
            if let Some(unsuccessful_txs) = advance_result {
                failed_txs.batch_send(unsuccessful_txs).await.unwrap();
            }
        }
    }
}
