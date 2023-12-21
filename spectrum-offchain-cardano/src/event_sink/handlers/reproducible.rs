use std::collections::{HashMap, HashSet};
use std::fmt::Debug;
use std::marker::PhantomData;
use std::sync::Arc;

use async_trait::async_trait;
use cml_multi_era::babbage::{BabbageTransaction, BabbageTransactionOutput};
use futures::{Sink, SinkExt};
use log::trace;
use tokio::sync::Mutex;

use cardano_chain_sync::data::LedgerTxEvent;
use cardano_mempool_sync::data::MempoolUpdate;
use spectrum_cardano_lib::OutputRef;
use spectrum_offchain::box_resolver::persistence::EntityRepo;
use spectrum_offchain::combinators::Ior;
use spectrum_offchain::data::unique_entity::{Confirmed, EitherMod, StateUpdate, Unconfirmed};
use spectrum_offchain::data::LiquiditySource;
use spectrum_offchain::event_sink::event_handler::EventHandler;
use spectrum_offchain::ledger::TryFromLedger;

use spectrum_cardano_lib::hash::hash_transaction_canonical;

pub struct ConfirmedUpdateHandler<TSink, TEntity, TRepo>
where
    TEntity: LiquiditySource,
{
    pub topic: TSink,
    pub entities: Arc<Mutex<TRepo>>,
    pub pd: PhantomData<TEntity>,
}

impl<TSink, TEntity, TRepo> ConfirmedUpdateHandler<TSink, TEntity, TRepo>
where
    TEntity: LiquiditySource + TryFromLedger<BabbageTransactionOutput, OutputRef> + Clone,
    TEntity::StableId: Clone,
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
    tx: BabbageTransaction,
) -> Vec<Ior<TEntity, TEntity>>
where
    TEntity: LiquiditySource + TryFromLedger<BabbageTransactionOutput, OutputRef> + Clone,
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
        if let Some(entity) = TEntity::try_from_ledger(o.clone(), o_ref) {
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

#[async_trait(?Send)]
impl<TSink, TEntity, TRepo> EventHandler<LedgerTxEvent<BabbageTransaction>>
    for ConfirmedUpdateHandler<TSink, TEntity, TRepo>
where
    TSink: Sink<EitherMod<StateUpdate<TEntity>>> + Unpin,
    TEntity: LiquiditySource + TryFromLedger<BabbageTransactionOutput, OutputRef> + Clone + Debug,
    TEntity::StableId: Clone,
    TEntity::Version: From<OutputRef> + Copy,
    TRepo: EntityRepo<TEntity>,
{
    async fn try_handle(
        &mut self,
        ev: LedgerTxEvent<BabbageTransaction>,
    ) -> Option<LedgerTxEvent<BabbageTransaction>> {
        let res = match ev {
            LedgerTxEvent::TxApplied { tx, slot } => {
                let transitions = extract_transitions(Arc::clone(&self.entities), tx.clone()).await;
                let num_transitions = transitions.len();
                let is_success = num_transitions > 0;
                for tr in transitions {
                    let _ = self
                        .topic
                        .feed(EitherMod::Confirmed(Confirmed(StateUpdate::Transition(tr))))
                        .await;
                }
                if is_success {
                    trace!(target: "offchain_lm", "[{}] entities parsed from applied tx", num_transitions);
                    None
                } else {
                    Some(LedgerTxEvent::TxApplied { tx, slot })
                }
            }
            LedgerTxEvent::TxUnapplied(tx) => {
                let transitions = extract_transitions(Arc::clone(&self.entities), tx.clone()).await;
                let num_transitions = transitions.len();
                let is_success = num_transitions > 0;
                for tr in transitions {
                    let _ = self
                        .topic
                        .feed(EitherMod::Confirmed(Confirmed(StateUpdate::TransitionRollback(
                            tr.swap(),
                        ))))
                        .await;
                }
                if is_success {
                    trace!(target: "offchain_lm", "[{}] entities parsed from unapplied tx", num_transitions);
                    None
                } else {
                    Some(LedgerTxEvent::TxUnapplied(tx))
                }
            }
        };
        let _ = self.topic.flush().await;
        res
    }
}

pub struct UnconfirmedUpdateHandler<TSink, TEntity, TRepo>
where
    TEntity: LiquiditySource,
{
    pub topic: TSink,
    pub entities: Arc<Mutex<TRepo>>,
    pub pd: PhantomData<TEntity>,
}

impl<TSink, TEntity, TRepo> UnconfirmedUpdateHandler<TSink, TEntity, TRepo>
where
    TEntity: LiquiditySource + TryFromLedger<BabbageTransactionOutput, OutputRef> + Clone,
    TEntity::StableId: Clone,
{
    pub fn new(topic: TSink, entities: Arc<Mutex<TRepo>>) -> Self {
        Self {
            topic,
            entities,
            pd: Default::default(),
        }
    }
}

#[async_trait(?Send)]
impl<TSink, TEntity, TRepo> EventHandler<MempoolUpdate<BabbageTransaction>>
    for UnconfirmedUpdateHandler<TSink, TEntity, TRepo>
where
    TSink: Sink<EitherMod<StateUpdate<TEntity>>> + Unpin,
    TEntity: LiquiditySource + TryFromLedger<BabbageTransactionOutput, OutputRef> + Clone + Debug,
    TEntity::StableId: Clone,
    TEntity::Version: From<OutputRef> + Copy,
    TRepo: EntityRepo<TEntity>,
{
    async fn try_handle(
        &mut self,
        ev: MempoolUpdate<BabbageTransaction>,
    ) -> Option<MempoolUpdate<BabbageTransaction>> {
        let res = match ev {
            MempoolUpdate::TxAccepted(tx) => {
                let transitions = extract_transitions(Arc::clone(&self.entities), tx.clone()).await;
                let is_success = !transitions.is_empty();
                for tr in transitions {
                    let _ = self
                        .topic
                        .feed(EitherMod::Unconfirmed(Unconfirmed(StateUpdate::Transition(tr))))
                        .await;
                }
                if is_success {
                    Some(MempoolUpdate::TxAccepted(tx))
                } else {
                    None
                }
            }
        };
        let _ = self.topic.flush().await;
        res
    }
}
