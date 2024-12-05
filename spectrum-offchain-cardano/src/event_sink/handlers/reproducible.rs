use std::collections::{HashMap, HashSet};
use std::fmt::Debug;
use std::marker::PhantomData;
use std::sync::Arc;

use async_trait::async_trait;
use cml_chain::transaction::{Transaction, TransactionOutput};
use cml_multi_era::babbage::{BabbageTransaction, BabbageTransactionOutput};
use futures::{Sink, SinkExt};
use log::trace;
use tokio::sync::Mutex;

use cardano_chain_sync::data::LedgerTxEvent;
use cardano_mempool_sync::data::MempoolUpdate;
use spectrum_cardano_lib::hash::hash_transaction_canonical;
use spectrum_cardano_lib::OutputRef;
use spectrum_offchain::box_resolver::persistence::EntityRepo;
use spectrum_offchain::data::ior::Ior;
use spectrum_offchain::domain::event::{Channel, Confirmed, StateUpdate, Unconfirmed};
use spectrum_offchain::domain::EntitySnapshot;
use spectrum_offchain::event_sink::event_handler::EventHandler;
use spectrum_offchain::ledger::TryFromLedger;

/// Trivial handler for confirmed updates.
pub struct ConfirmedUpdateHandler<TSink, TEntity, TRepo>
where
    TEntity: EntitySnapshot,
{
    pub topic: TSink,
    pub entities: Arc<Mutex<TRepo>>,
    pub pd: PhantomData<TEntity>,
}

impl<TSink, TEntity, TRepo> ConfirmedUpdateHandler<TSink, TEntity, TRepo>
where
    TEntity: EntitySnapshot,
{
    pub fn new(topic: TSink, entities: Arc<Mutex<TRepo>>) -> Self {
        Self {
            topic,
            entities,
            pd: Default::default(),
        }
    }
}

async fn extract_transitions<TEntity, TRepo>(
    entities: Arc<Mutex<TRepo>>,
    tx: Transaction,
) -> Vec<Ior<TEntity, TEntity>>
where
    TEntity: EntitySnapshot + TryFromLedger<TransactionOutput, OutputRef> + Clone,
    TEntity::StableId: Clone,
    TEntity::Version: From<OutputRef> + Copy,
    TRepo: EntityRepo<TEntity>,
{
    let mut consumed_entities = HashMap::<TEntity::StableId, TEntity>::new();
    for i in &tx.body.inputs {
        let state_id = TEntity::Version::from(OutputRef::from((i.transaction_id, i.index)));
        let entities = entities.lock().await;
        if entities.may_exist(state_id).await {
            if let Some(entity) = entities.get_state(state_id).await {
                let entity_id = entity.stable_id();
                consumed_entities.insert(entity_id, entity);
            }
        }
    }
    let mut created_entities = HashMap::<TEntity::StableId, TEntity>::new();
    let tx_hash = hash_transaction_canonical(&tx.body);
    for (i, o) in tx.body.outputs.iter().enumerate() {
        let o_ref = OutputRef::from((tx_hash, i as u64));
        if let Some(entity) = TEntity::try_from_ledger(o, &o_ref) {
            let entity_id = entity.stable_id();
            created_entities.insert(entity_id.clone(), entity);
        }
    }
    let consumed_keys = consumed_entities.keys().cloned().collect::<HashSet<_>>();
    let created_keys = created_entities.keys().cloned().collect::<HashSet<_>>();

    consumed_keys
        .union(&created_keys)
        .flat_map(|k| {
            Ior::try_from((consumed_entities.remove(k), created_entities.remove(k)))
                .map(|x| vec![x])
                .unwrap_or(Vec::new())
        })
        .collect()
}

#[async_trait]
impl<TSink, TEntity, TRepo> EventHandler<LedgerTxEvent<Transaction>>
    for ConfirmedUpdateHandler<TSink, TEntity, TRepo>
where
    TSink: Sink<Channel<StateUpdate<TEntity>>> + Send + Unpin,
    TEntity: EntitySnapshot + TryFromLedger<TransactionOutput, OutputRef> + Clone + Send + Debug,
    TEntity::StableId: Clone,
    TEntity::Version: From<OutputRef> + Copy,
    TRepo: EntityRepo<TEntity> + Send,
{
    async fn try_handle(&mut self, ev: LedgerTxEvent<Transaction>) -> Option<LedgerTxEvent<Transaction>> {
        let res = match ev {
            LedgerTxEvent::TxApplied { tx, slot } => {
                let transitions = extract_transitions(Arc::clone(&self.entities), tx.clone()).await;
                let num_transitions = transitions.len();
                let is_success = num_transitions > 0;
                for tr in transitions {
                    let _ = self
                        .topic
                        .feed(Channel::Ledger(Confirmed(StateUpdate::Transition(tr))))
                        .await;
                }
                if is_success {
                    trace!(target: "offchain_lm", "[{}] entities parsed from applied tx", num_transitions);
                    None
                } else {
                    Some(LedgerTxEvent::TxApplied { tx, slot })
                }
            }
            LedgerTxEvent::TxUnapplied { tx, slot } => {
                let transitions = extract_transitions(Arc::clone(&self.entities), tx.clone()).await;
                let num_transitions = transitions.len();
                let is_success = num_transitions > 0;
                for tr in transitions {
                    let _ = self
                        .topic
                        .feed(Channel::Ledger(Confirmed(StateUpdate::TransitionRollback(
                            tr.swap(),
                        ))))
                        .await;
                }
                if is_success {
                    trace!(target: "offchain_lm", "[{}] entities parsed from unapplied tx", num_transitions);
                    None
                } else {
                    Some(LedgerTxEvent::TxUnapplied { tx, slot })
                }
            }
        };
        let _ = self.topic.flush().await;
        res
    }
}

pub struct UnconfirmedUpdateHandler<TSink, TEntity, TRepo>
where
    TEntity: EntitySnapshot,
{
    pub topic: TSink,
    pub entities: Arc<Mutex<TRepo>>,
    pub pd: PhantomData<TEntity>,
}

impl<TSink, TEntity, TRepo> UnconfirmedUpdateHandler<TSink, TEntity, TRepo>
where
    TEntity: EntitySnapshot,
{
    pub fn new(topic: TSink, entities: Arc<Mutex<TRepo>>) -> Self {
        Self {
            topic,
            entities,
            pd: Default::default(),
        }
    }
}

#[async_trait]
impl<TSink, TEntity, TRepo> EventHandler<MempoolUpdate<Transaction>>
    for UnconfirmedUpdateHandler<TSink, TEntity, TRepo>
where
    TSink: Sink<Channel<StateUpdate<TEntity>>> + Unpin + Send,
    TEntity: EntitySnapshot + TryFromLedger<TransactionOutput, OutputRef> + Clone + Send + Debug,
    TEntity::StableId: Clone,
    TEntity::Version: From<OutputRef> + Copy,
    TRepo: EntityRepo<TEntity> + Send,
{
    async fn try_handle(&mut self, ev: MempoolUpdate<Transaction>) -> Option<MempoolUpdate<Transaction>> {
        let res = match ev {
            MempoolUpdate::TxAccepted(tx) => {
                let transitions = extract_transitions(Arc::clone(&self.entities), tx.clone()).await;
                let is_success = !transitions.is_empty();
                for tr in transitions {
                    let _ = self
                        .topic
                        .feed(Channel::Mempool(Unconfirmed(StateUpdate::Transition(tr))))
                        .await;
                }
                if is_success {
                    None
                } else {
                    Some(MempoolUpdate::TxAccepted(tx))
                }
            }
        };
        let _ = self.topic.flush().await;
        res
    }
}
