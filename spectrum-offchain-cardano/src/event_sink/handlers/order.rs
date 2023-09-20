use std::marker::PhantomData;
use std::sync::Arc;

use async_trait::async_trait;
use cml_chain::crypto::hash::hash_transaction;
use cml_chain::transaction::{Transaction, TransactionOutput};
use futures::{Sink, SinkExt};
use log::info;
use tokio::sync::Mutex;

use cardano_chain_sync::data::LedgerTxEvent;
use cardano_mempool_sync::data::MempoolUpdate;
use spectrum_cardano_lib::OutputRef;
use spectrum_offchain::data::order::{OrderLink, OrderUpdate};
use spectrum_offchain::data::SpecializedOrder;
use spectrum_offchain::event_sink::event_handler::EventHandler;
use spectrum_offchain::ledger::TryFromLedger;

use crate::event_sink::handlers::order::registry::HotOrderRegistry;

pub mod registry;

pub struct ClassicalOrderUpdatesHandler<TSink, TOrd, TRegistry> {
    pub topic: TSink,
    pub registry: Arc<Mutex<TRegistry>>,
    pub pd: PhantomData<TOrd>,
}

impl<TSink, TOrd, TRegistry> ClassicalOrderUpdatesHandler<TSink, TOrd, TRegistry> {
    pub fn new(topic: TSink, registry: Arc<Mutex<TRegistry>>) -> Self {
        Self {
            topic,
            registry,
            pd: Default::default(),
        }
    }

    async fn handle_applied_tx<F, R>(&mut self, tx: Transaction, on_failure: F) -> Option<R>
    where
        TSink: Sink<OrderUpdate<TOrd, OrderLink<TOrd>>> + Unpin,
        TOrd: SpecializedOrder + TryFromLedger<TransactionOutput, OutputRef>,
        TOrd::TOrderId: From<OutputRef> + Copy,
        TRegistry: HotOrderRegistry<TOrd>,
        F: FnOnce(Transaction) -> R,
    {
        let mut is_success = false;
        for i in &tx.body.inputs {
            let maybe_order_link = {
                let order_id = TOrd::TOrderId::from(OutputRef::from((i.transaction_id, i.index)));
                let mut registry = self.registry.lock().await;
                registry.deregister(order_id)
            };
            if let Some(order_link) = maybe_order_link {
                is_success = true;
                let _ = self.topic.feed(OrderUpdate::OrderEliminated(order_link)).await;
                break;
            }
        }
        if !is_success {
            let tx_hash = hash_transaction(&tx.body);
            // no point in searching for new orders in execution tx
            for (i, o) in tx.body.outputs.iter().enumerate() {
                let o_ref = OutputRef::from((tx_hash, i as u64));
                if let Some(order) = TOrd::try_from_ledger(o.clone(), o_ref) {
                    is_success = true;
                    {
                        let mut registry = self.registry.lock().await;
                        registry.register(OrderLink {
                            order_id: order.get_self_ref(),
                            pool_id: order.get_pool_ref(),
                        });
                    };
                    let _ = self.topic.feed(OrderUpdate::NewOrder(order)).await;
                    info!(target: "offchain", "Observing new order");
                }
            }
        }
        if is_success {
            return None;
        }
        Some(on_failure(tx))
    }

    async fn handle_unapplied_tx(&mut self, tx: Transaction) -> Option<LedgerTxEvent>
    where
        TSink: Sink<OrderUpdate<TOrd, OrderLink<TOrd>>> + Unpin,
        TOrd: SpecializedOrder + TryFromLedger<TransactionOutput, OutputRef>,
        TOrd::TOrderId: From<OutputRef> + Copy,
        TRegistry: HotOrderRegistry<TOrd>,
    {
        let mut is_success = false;
        let tx_hash = hash_transaction(&tx.body);
        for (i, _) in tx.body.outputs.iter().enumerate() {
            let maybe_order_link = {
                let o_ref = OutputRef::from((tx_hash, i as u64));
                let order_id = TOrd::TOrderId::from(o_ref);
                let mut registry = self.registry.lock().await;
                registry.deregister(order_id)
            };
            if let Some(order_link) = maybe_order_link {
                is_success = true;
                let _ = self.topic.feed(OrderUpdate::OrderEliminated(order_link)).await;
                break;
            }
        }
        if is_success {
            return None;
        }
        Some(LedgerTxEvent::TxUnapplied(tx))
    }
}

#[async_trait(?Send)]
impl<TSink, TOrd, TRegistry> EventHandler<LedgerTxEvent>
    for ClassicalOrderUpdatesHandler<TSink, TOrd, TRegistry>
where
    TSink: Sink<OrderUpdate<TOrd, OrderLink<TOrd>>> + Unpin,
    TOrd: SpecializedOrder + TryFromLedger<TransactionOutput, OutputRef>,
    TOrd::TOrderId: From<OutputRef> + Copy,
    TRegistry: HotOrderRegistry<TOrd>,
{
    async fn try_handle(&mut self, ev: LedgerTxEvent) -> Option<LedgerTxEvent> {
        let res = match ev {
            LedgerTxEvent::TxApplied(tx) => self.handle_applied_tx(tx, LedgerTxEvent::TxApplied).await,
            LedgerTxEvent::TxUnapplied(tx) => self.handle_unapplied_tx(tx).await,
        };
        let _ = self.topic.flush().await;
        res
    }
}

#[async_trait(?Send)]
impl<TSink, TOrd, TRegistry> EventHandler<MempoolUpdate<Transaction>>
    for ClassicalOrderUpdatesHandler<TSink, TOrd, TRegistry>
where
    TSink: Sink<OrderUpdate<TOrd, OrderLink<TOrd>>> + Unpin,
    TOrd: SpecializedOrder + TryFromLedger<TransactionOutput, OutputRef>,
    TOrd::TOrderId: From<OutputRef> + Copy,
    TRegistry: HotOrderRegistry<TOrd>,
{
    async fn try_handle(&mut self, ev: MempoolUpdate<Transaction>) -> Option<MempoolUpdate<Transaction>> {
        let res = match ev {
            MempoolUpdate::TxAccepted(tx) => self.handle_applied_tx(tx, MempoolUpdate::TxAccepted).await,
        };
        let _ = self.topic.flush().await;
        res
    }
}
