use std::collections::{HashMap, HashSet};
use std::fmt::Debug;
use std::hash::Hash;
use std::marker::PhantomData;
use std::sync::Arc;

use async_trait::async_trait;
use cml_multi_era::babbage::{BabbageTransaction, BabbageTransactionOutput};
use futures::{Sink, SinkExt};
use log::trace;
use tokio::sync::Mutex;

use cardano_chain_sync::data::LedgerTxEvent;
use cardano_mempool_sync::data::MempoolUpdate;
use spectrum_cardano_lib::hash::hash_transaction_canonical;
use spectrum_cardano_lib::OutputRef;
use spectrum_offchain::combinators::Ior;
use spectrum_offchain::data::unique_entity::{Confirmed, EitherMod, StateUpdate, Unconfirmed};
use spectrum_offchain::data::{EntitySnapshot, Tradable};
use spectrum_offchain::event_sink::event_handler::EventHandler;
use spectrum_offchain::ledger::TryFromLedger;
use spectrum_offchain::partitioning::Partitioned;

use crate::event_sink::entity_index::EntityIndex;

/// A handler for updates that routes resulted [Entity] updates
/// into different topics [Topic] according to partitioning key [PairId].
#[derive(Clone)]
pub struct PairUpdateHandler<const N: usize, PairId, Topic, Entity, Index>
where
    Entity: EntitySnapshot,
{
    pub topic: Partitioned<N, PairId, Topic>,
    /// Index of all non-consumed states of [Entity].
    pub index: Arc<Mutex<Index>>,
    pub pd: PhantomData<Entity>,
}

impl<const N: usize, PairId, Topic, Entity, Index> PairUpdateHandler<N, PairId, Topic, Entity, Index>
where
    Entity: EntitySnapshot + TryFromLedger<BabbageTransactionOutput, OutputRef> + Clone,
{
    pub fn new(topic: Partitioned<N, PairId, Topic>, index: Arc<Mutex<Index>>) -> Self {
        Self {
            topic,
            index,
            pd: Default::default(),
        }
    }
}

// todo: test for rollback processing correctness.
async fn extract_transitions<Entity, Index>(
    index: Arc<Mutex<Index>>,
    mut tx: BabbageTransaction,
) -> Result<Vec<Ior<Entity, Entity>>, BabbageTransaction>
where
    Entity: EntitySnapshot + TryFromLedger<BabbageTransactionOutput, OutputRef> + Clone,
    Entity::Version: From<OutputRef>,
    Index: EntityIndex<Entity>,
{
    let mut consumed_entities = HashMap::<Entity::StableId, Entity>::new();
    for i in &tx.body.inputs {
        let state_id = Entity::Version::from(OutputRef::from((i.transaction_id, i.index)));
        let mut index = index.lock().await;
        if index.exists(&state_id) {
            if let Some(entity) = index.take_state(&state_id) {
                let entity_id = entity.stable_id();
                consumed_entities.insert(entity_id, entity);
            }
        }
    }
    let mut produced_entities = HashMap::<Entity::StableId, Entity>::new();
    let tx_hash = hash_transaction_canonical(&tx.body);
    let mut ix = tx.body.outputs.len() - 1;
    while let Some(o) = tx.body.outputs.pop() {
        let o_ref = OutputRef::new(tx_hash, ix as u64);
        match Entity::try_from_ledger(&o, o_ref) {
            Some(entity) => {
                let entity_id = entity.stable_id();
                produced_entities.insert(entity_id, entity);
            }
            None => {
                tx.body.outputs.push(o);
            }
        }
        ix += 1;
    }
    // Preserve original ordering of outputs.
    tx.body.outputs.reverse();

    // Gather IDs of all recognized entities.
    let mut keys = HashSet::new();
    for k in consumed_entities.keys().chain(produced_entities.keys()) {
        keys.insert(*k);
    }

    // Match consumed versions with produced ones.
    let mut transitions = vec![];
    for k in keys.into_iter() {
        if let Ok(xa) = Ior::try_from((consumed_entities.remove(&k), produced_entities.remove(&k))) {
            transitions.push(xa);
        }
    }

    if transitions.is_empty() {
        return Err(tx);
    }
    Ok(transitions)
}

fn pair_id_of<T: EntitySnapshot + Tradable>(xa: &Ior<T, T>) -> T::PairId {
    match xa {
        Ior::Left(o) => o.pair_id(),
        Ior::Right(o) => o.pair_id(),
        Ior::Both(o, _) => o.pair_id(),
    }
}

#[async_trait(?Send)]
impl<const N: usize, PairId, Topic, Entity, Index> EventHandler<LedgerTxEvent<BabbageTransaction>>
    for PairUpdateHandler<N, PairId, Topic, Entity, Index>
where
    PairId: Copy + Hash,
    Topic: Sink<(PairId, EitherMod<StateUpdate<Entity>>)> + Unpin,
    Topic::Error: Debug,
    Entity: EntitySnapshot
        + Tradable<PairId = PairId>
        + TryFromLedger<BabbageTransactionOutput, OutputRef>
        + Clone
        + Debug,
    Entity::Version: From<OutputRef>,
    Index: EntityIndex<Entity>,
{
    async fn try_handle(
        &mut self,
        ev: LedgerTxEvent<BabbageTransaction>,
    ) -> Option<LedgerTxEvent<BabbageTransaction>> {
        match ev {
            LedgerTxEvent::TxApplied { tx, slot } => {
                match extract_transitions(Arc::clone(&self.index), tx).await {
                    Ok(transitions) => {
                        trace!("[{}] entities parsed from applied tx", transitions.len());
                        for tr in transitions {
                            let pair = pair_id_of(&tr);
                            let topic = self.topic.get_mut(pair);
                            topic
                                .feed((pair, EitherMod::Confirmed(Confirmed(StateUpdate::Transition(tr)))))
                                .await
                                .expect("Channel is closed");
                            topic.flush().await.expect("Failed to commit message");
                        }
                        None
                    }
                    Err(tx) => Some(LedgerTxEvent::TxApplied { tx, slot }),
                }
            }
            LedgerTxEvent::TxUnapplied(tx) => match extract_transitions(Arc::clone(&self.index), tx).await {
                Ok(transitions) => {
                    trace!(target: "offchain_lm", "[{}] entities parsed from unapplied tx", transitions.len());
                    for tr in transitions {
                        let pair = pair_id_of(&tr);
                        let topic = self.topic.get_mut(pair);
                        topic
                            .feed((
                                pair,
                                EitherMod::Confirmed(Confirmed(StateUpdate::TransitionRollback(tr))),
                            ))
                            .await
                            .expect("Channel is closed");
                        topic.flush().await.expect("Failed to commit message");
                    }
                    None
                }
                Err(tx) => Some(LedgerTxEvent::TxUnapplied(tx)),
            },
        }
    }
}

#[async_trait(?Send)]
impl<const N: usize, PairId, Topic, Entity, Index> EventHandler<MempoolUpdate<BabbageTransaction>>
    for PairUpdateHandler<N, PairId, Topic, Entity, Index>
where
    PairId: Copy + Hash,
    Topic: Sink<(PairId, EitherMod<StateUpdate<Entity>>)> + Unpin,
    Topic::Error: Debug,
    Entity: EntitySnapshot
        + Tradable<PairId = PairId>
        + TryFromLedger<BabbageTransactionOutput, OutputRef>
        + Clone
        + Debug,
    Entity::Version: From<OutputRef>,
    Index: EntityIndex<Entity>,
{
    async fn try_handle(
        &mut self,
        ev: MempoolUpdate<BabbageTransaction>,
    ) -> Option<MempoolUpdate<BabbageTransaction>> {
        match ev {
            MempoolUpdate::TxAccepted(tx) => match extract_transitions(Arc::clone(&self.index), tx).await {
                Ok(transitions) => {
                    for tr in transitions {
                        let pair = pair_id_of(&tr);
                        let topic = self.topic.get_mut(pair);
                        topic
                            .feed((
                                pair,
                                EitherMod::Unconfirmed(Unconfirmed(StateUpdate::Transition(tr))),
                            ))
                            .await
                            .expect("Channel is closed");
                        topic.flush().await.expect("Failed to commit message");
                    }
                    None
                }
                Err(tx) => Some(MempoolUpdate::TxAccepted(tx)),
            },
        }
    }
}
