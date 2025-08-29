use std::collections::hash_map::Entry;
use std::collections::{HashMap, HashSet, VecDeque};
use std::fmt::{Debug, Display};
use std::hash::Hash;
use std::marker::PhantomData;
use std::sync::Arc;

use crate::event_sink::context::EventContext;
use crate::event_sink::entity_index::TradableEntityIndex;
use crate::event_sink::order_index::KvIndex;
use crate::event_sink::tx_view::TxViewMut;
use async_trait::async_trait;
use bloom_offchain::execution_engine::funding_effect::FundingEvent;
use cardano_chain_sync::data::LedgerTxEvent;
use cardano_mempool_sync::data::MempoolUpdate;
use cml_chain::address::{Address, BaseAddress, EnterpriseAddress};
use cml_chain::certs::Credential;
use cml_chain::transaction::TransactionOutput;
use cml_core::Slot;
use cml_crypto::BlockHeaderHash;
use either::Either;
use futures::Sink;
use log::trace;
use spectrum_cardano_lib::output::FinalizedTxOut;
use spectrum_cardano_lib::OutputRef;
use spectrum_offchain::data::ior::Ior;
use spectrum_offchain::data::small_vec::SmallVec;
use spectrum_offchain::domain::event::{Channel, Transition};
use spectrum_offchain::domain::order::{OrderUpdate, SpecializedOrder};
use spectrum_offchain::domain::EntitySnapshot;
use spectrum_offchain::domain::Tradable;
use spectrum_offchain::event_sink::event_handler::EventHandler;
use spectrum_offchain::ledger::TryFromLedger;
use spectrum_offchain::partitioning::Partitioned;
use spectrum_offchain::sink::{BatchSinkExt, KeyedBatchSinkExt};
use spectrum_offchain_cardano::funding::FundingAddresses;
use spectrum_offchain_cardano::handler_context::AddedPaymentDestinations;
use tokio::sync::{Mutex, MutexGuard};

#[derive(Copy, Clone, Eq, PartialEq, Debug)]
pub struct LedgerCx {
    pub block_hash: BlockHeaderHash,
    pub slot: Slot,
}

impl LedgerCx {
    pub fn new(block_hash: BlockHeaderHash, slot: Slot) -> Self {
        Self { block_hash, slot }
    }
}

#[derive(Clone)]
pub struct FundingEventHandler<const N: usize, Topic, Index> {
    pub topic: Partitioned<N, usize, Topic>,
    pub funding_addresses: FundingAddresses<N>,
    /// UTxO we should not use for funding.
    pub skip_set: OutputRef,
    pub index: Arc<Mutex<Index>>,
}

impl<const N: usize, Topic, Index> FundingEventHandler<N, Topic, Index> {
    pub fn new(
        topic: Partitioned<N, usize, Topic>,
        funding_addresses: FundingAddresses<N>,
        skip_set: OutputRef,
        index: Arc<Mutex<Index>>,
    ) -> Self {
        Self {
            topic,
            funding_addresses,
            skip_set,
            index,
        }
    }
}

async fn extract_funding_events<const N: usize, Index>(
    mut tx: TxViewMut,
    funding_addresses: FundingAddresses<N>,
    skip_set: OutputRef,
    index: Arc<Mutex<Index>>,
) -> Result<(Vec<(usize, FundingEvent<FinalizedTxOut>)>, TxViewMut), TxViewMut>
where
    Index: KvIndex<OutputRef, (usize, FinalizedTxOut)>,
{
    let num_outputs = tx.outputs.len();
    if num_outputs == 0 {
        return Err(tx);
    }
    let mut consumed_utxos = vec![];
    for i in &tx.inputs {
        let oref = OutputRef::from((i.transaction_id, i.index));
        if let Some(utxo) = index.lock().await.get(&oref) {
            consumed_utxos.push(utxo);
        }
    }
    let mut non_processed_outputs = VecDeque::new();
    let mut produced_utxos = vec![];
    while let Some((ix, o)) = tx.outputs.pop() {
        let o_ref = OutputRef::new(tx.hash, ix as u64);
        if let Some(part) = funding_addresses.partition_by_address(o.address()) {
            if o_ref != skip_set {
                let txo = FinalizedTxOut(o, o_ref);
                produced_utxos.push((part, txo));
            }
        } else {
            non_processed_outputs.push_front((ix, o));
        }
    }
    // Preserve non-processed outputs in original ordering.
    tx.outputs = non_processed_outputs.into();
    let events = consumed_utxos
        .into_iter()
        .map(|(pt, utxo)| (pt, FundingEvent::Consumed(utxo)))
        .chain(
            produced_utxos
                .into_iter()
                .map(|(pt, utxo)| (pt, FundingEvent::Produced(utxo))),
        )
        .collect::<Vec<_>>();

    if events.is_empty() {
        return Err(tx);
    }
    Ok((events, tx))
}

fn index_funding_event<Index>(index: &mut MutexGuard<Index>, part: usize, tr: &FundingEvent<FinalizedTxOut>)
where
    Index: KvIndex<OutputRef, (usize, FinalizedTxOut)>,
{
    match &tr {
        FundingEvent::Consumed(consumed) => {
            index.register_for_eviction(consumed.reference());
        }
        FundingEvent::Produced(produced) => {
            index.put(produced.reference(), (part, produced.clone()));
        }
    }
}

#[async_trait]
impl<const N: usize, Topic, Index> EventHandler<LedgerTxEvent<TxViewMut>>
    for FundingEventHandler<N, Topic, Index>
where
    Topic: Sink<FundingEvent<FinalizedTxOut>> + Unpin + Send,
    Topic::Error: Debug,
    Index: KvIndex<OutputRef, (usize, FinalizedTxOut)> + Send,
{
    async fn try_handle(&mut self, ev: LedgerTxEvent<TxViewMut>) -> Option<LedgerTxEvent<TxViewMut>> {
        let mut events_by_part: HashMap<usize, Vec<FundingEvent<FinalizedTxOut>>> = HashMap::new();
        let remainder = match ev {
            LedgerTxEvent::TxApplied {
                tx,
                slot,
                block_number,
                block_hash,
            } => {
                match extract_funding_events(
                    tx,
                    self.funding_addresses.clone(),
                    self.skip_set,
                    self.index.clone(),
                )
                .await
                {
                    Ok((events, tx)) => {
                        trace!("{} funding boxes found in applied TX", events.len());
                        let mut index = self.index.lock().await;
                        index.run_eviction();
                        for (pt, event) in events {
                            index_funding_event(&mut index, pt, &event);
                            match events_by_part.entry(pt) {
                                Entry::Occupied(mut entry) => {
                                    entry.get_mut().push(event);
                                }
                                Entry::Vacant(entry) => {
                                    entry.insert(vec![event]);
                                }
                            }
                        }
                        Some(LedgerTxEvent::TxApplied {
                            tx,
                            slot,
                            block_number,
                            block_hash,
                        })
                    }
                    Err(tx) => Some(LedgerTxEvent::TxApplied {
                        tx,
                        slot,
                        block_number,
                        block_hash,
                    }),
                }
            }
            LedgerTxEvent::TxUnapplied {
                tx,
                slot,
                block_number,
                block_hash,
            } => {
                match extract_funding_events(
                    tx,
                    self.funding_addresses.clone(),
                    self.skip_set,
                    self.index.clone(),
                )
                .await
                {
                    Ok((events, tx)) => {
                        trace!("{} funding boxes found in unapplied TX", events.len());
                        let mut index = self.index.lock().await;
                        index.run_eviction();
                        for (pt, event) in events {
                            let event = event.inverse();
                            index_funding_event(&mut index, pt, &event);
                            match events_by_part.entry(pt) {
                                Entry::Occupied(mut entry) => {
                                    entry.get_mut().push(event);
                                }
                                Entry::Vacant(entry) => {
                                    entry.insert(vec![event]);
                                }
                            }
                        }
                        Some(LedgerTxEvent::TxUnapplied {
                            tx,
                            slot,
                            block_number,
                            block_hash,
                        })
                    }
                    Err(tx) => Some(LedgerTxEvent::TxUnapplied {
                        tx,
                        slot,
                        block_number,
                        block_hash,
                    }),
                }
            }
        };
        for (pt, events) in events_by_part {
            let num_updates = events.len();
            let topic = self.topic.get_by_id_mut(pt);
            topic.batch_send(events).await.expect("Failed to submit updates");
            trace!("{} funding events from ledger were commited", num_updates);
        }
        remainder
    }
}

#[async_trait]
impl<const N: usize, Topic, Index> EventHandler<MempoolUpdate<TxViewMut>>
    for FundingEventHandler<N, Topic, Index>
where
    Topic: Sink<FundingEvent<FinalizedTxOut>> + Unpin + Send,
    Topic::Error: Debug,
    Index: KvIndex<OutputRef, (usize, FinalizedTxOut)> + Send,
{
    async fn try_handle(&mut self, ev: MempoolUpdate<TxViewMut>) -> Option<MempoolUpdate<TxViewMut>> {
        let mut events_by_part: HashMap<usize, Vec<FundingEvent<FinalizedTxOut>>> = HashMap::new();
        let remainder = match ev {
            MempoolUpdate::TxAccepted(tx) => {
                match extract_funding_events(
                    tx,
                    self.funding_addresses.clone(),
                    self.skip_set,
                    self.index.clone(),
                )
                .await
                {
                    Ok((events, tx)) => {
                        trace!("{} funding boxes found in accepted TX", events.len());
                        let mut index = self.index.lock().await;
                        index.run_eviction();
                        for (pt, event) in events {
                            index_funding_event(&mut index, pt, &event);
                            match events_by_part.entry(pt) {
                                Entry::Occupied(mut entry) => {
                                    entry.get_mut().push(event);
                                }
                                Entry::Vacant(entry) => {
                                    entry.insert(vec![event]);
                                }
                            }
                        }
                        Some(MempoolUpdate::TxAccepted(tx))
                    }
                    Err(tx) => Some(MempoolUpdate::TxAccepted(tx)),
                }
            }
            MempoolUpdate::TxDropped(tx) => {
                match extract_funding_events(
                    tx,
                    self.funding_addresses.clone(),
                    self.skip_set,
                    self.index.clone(),
                )
                .await
                {
                    Ok((events, tx)) => {
                        trace!("{} funding boxes found in dropped TX", events.len());
                        let mut index = self.index.lock().await;
                        index.run_eviction();
                        for (pt, event) in events {
                            let event = event.inverse();
                            index_funding_event(&mut index, pt, &event);
                            match events_by_part.entry(pt) {
                                Entry::Occupied(mut entry) => {
                                    entry.get_mut().push(event);
                                }
                                Entry::Vacant(entry) => {
                                    entry.insert(vec![event]);
                                }
                            }
                        }
                        Some(MempoolUpdate::TxDropped(tx))
                    }
                    Err(tx) => Some(MempoolUpdate::TxDropped(tx)),
                }
            }
        };
        for (pt, events) in events_by_part {
            let num_updates = events.len();
            let topic = self.topic.get_by_id_mut(pt);
            topic.batch_send(events).await.expect("Failed to submit updates");
            trace!("{} funding events from mempool were commited", num_updates);
        }
        remainder
    }
}

/// A handler for updates that routes resulted [Entity] updates
/// into different topics [Topic] according to partitioning key [PairId].
#[derive(Clone)]
pub struct PairUpdateHandler<const N: usize, PairId, Topic, Entity, Index, Proto, Ctx> {
    pub topic: Partitioned<N, PairId, Topic>,
    /// Index of all non-consumed states of [Entity].
    pub index: Arc<Mutex<Index>>,
    pub context_proto: Proto,
    pub pd: PhantomData<Entity>,
    pub context: PhantomData<Ctx>,
}

impl<const N: usize, PairId, Topic, Entity, Index, Proto, Ctx>
    PairUpdateHandler<N, PairId, Topic, Entity, Index, Proto, Ctx>
{
    pub fn new(topic: Partitioned<N, PairId, Topic>, index: Arc<Mutex<Index>>, context_proto: Proto) -> Self {
        Self {
            topic,
            index,
            context_proto,
            pd: Default::default(),
            context: Default::default(),
        }
    }
}

#[derive(Clone)]
pub struct SpecializedHandler<H, OrderIndex, Pool, K, OpCtx> {
    general_handler: H,
    order_index: Arc<Mutex<OrderIndex>>,
    pd0: PhantomData<Pool>,
    pd1: PhantomData<K>,
    pd2: PhantomData<OpCtx>,
}

impl<H, OrderIndex, Pool, Ctx, OpCtx> SpecializedHandler<H, OrderIndex, Pool, Ctx, OpCtx> {
    pub fn new(general_handler: H, order_index: Arc<Mutex<OrderIndex>>) -> Self {
        Self {
            general_handler,
            order_index,
            pd0: PhantomData,
            pd1: PhantomData,
            pd2: PhantomData,
        }
    }
}

#[async_trait]
impl<const N: usize, PairId, Topic, Pool, Order, PoolIndex, OrderIndex, K, Proto, Ctx>
    EventHandler<LedgerTxEvent<TxViewMut>>
    for SpecializedHandler<
        PairUpdateHandler<N, PairId, Topic, Order, PoolIndex, Proto, Ctx>,
        OrderIndex,
        Pool,
        K,
        Ctx,
    >
where
    Proto: Copy + Send,
    Ctx: From<(Proto, EventContext<K>)> + Send,
    PairId: Copy + Hash + Eq + Send,
    Topic: Sink<(PairId, Channel<OrderUpdate<Order, Order>, LedgerCx>)> + Send + Unpin,
    Topic::Error: Debug,
    Pool: EntitySnapshot + Tradable<PairId = PairId> + Send,
    Order: SpecializedOrder<TPoolId = Pool::StableId>
        + TryFromLedger<TransactionOutput, Ctx>
        + Clone
        + Debug
        + Send,
    Order::TOrderId: From<OutputRef> + Display,
    OrderIndex: KvIndex<Order::TOrderId, Order> + Send,
    PoolIndex: TradableEntityIndex<Pool> + Send,
    K: Send + Copy,
{
    async fn try_handle(&mut self, ev: LedgerTxEvent<TxViewMut>) -> Option<LedgerTxEvent<TxViewMut>> {
        let mut updates: HashMap<PairId, Vec<Channel<OrderUpdate<Order, Order>, LedgerCx>>> = HashMap::new();
        let remainder = match ev {
            LedgerTxEvent::TxApplied {
                tx,
                slot,
                block_number,
                block_hash,
            } => {
                match extract_atomic_transitions(
                    Arc::clone(&self.order_index),
                    self.general_handler.context_proto,
                    tx,
                )
                .await
                {
                    Ok((transitions, tx)) => {
                        trace!("{} entities found in applied TX", transitions.len());
                        let pool_index = self.general_handler.index.lock().await;
                        let mut index = self.order_index.lock().await;
                        index.run_eviction();
                        let cx = LedgerCx::new(block_hash, slot);
                        for tr in transitions {
                            if let Some(pair) = pool_index.pair_of(&pool_ref_of(&tr)) {
                                index_atomic_transition(&mut index, &tr);
                                let upd = Channel::ledger(tr.into(), cx);
                                match updates.entry(pair) {
                                    Entry::Occupied(mut entry) => {
                                        entry.get_mut().push(upd);
                                    }
                                    Entry::Vacant(entry) => {
                                        entry.insert(vec![upd]);
                                    }
                                }
                            }
                        }
                        Some(LedgerTxEvent::TxApplied {
                            tx,
                            slot,
                            block_number,
                            block_hash,
                        })
                    }
                    Err(tx) => Some(LedgerTxEvent::TxApplied {
                        tx,
                        slot,
                        block_number,
                        block_hash,
                    }),
                }
            }
            LedgerTxEvent::TxUnapplied {
                tx,
                slot,
                block_number,
                block_hash,
            } => {
                match extract_atomic_transitions(
                    Arc::clone(&self.order_index),
                    self.general_handler.context_proto,
                    tx,
                )
                .await
                {
                    Ok((transitions, tx)) => {
                        trace!("{} entities found in unapplied TX", transitions.len());
                        let mut index = self.order_index.lock().await;
                        let pool_index = self.general_handler.index.lock().await;
                        let cx = LedgerCx::new(block_hash, slot);
                        index.run_eviction();
                        for tr in transitions {
                            if let Some(pair) = pool_index.pair_of(&pool_ref_of(&tr)) {
                                let inverse_tr = tr.flip();
                                index_atomic_transition(&mut index, &inverse_tr);
                                let upd = Channel::ledger(inverse_tr.into(), cx);
                                match updates.entry(pair) {
                                    Entry::Occupied(mut entry) => {
                                        entry.get_mut().push(upd);
                                    }
                                    Entry::Vacant(entry) => {
                                        entry.insert(vec![upd]);
                                    }
                                }
                            }
                        }
                        Some(LedgerTxEvent::TxUnapplied {
                            tx,
                            slot,
                            block_number,
                            block_hash,
                        })
                    }
                    Err(tx) => Some(LedgerTxEvent::TxUnapplied {
                        tx,
                        slot,
                        block_number,
                        block_hash,
                    }),
                }
            }
        };
        for (pair, updates_by_pair) in updates {
            let num_updates = updates_by_pair.len();
            let topic = self.general_handler.topic.get_mut(pair);
            topic
                .batch_send_by_key(pair, updates_by_pair)
                .await
                .expect("Failed to submit updates");
            trace!("{} special updates commited", num_updates);
        }
        remainder
    }
}

#[async_trait]
impl<const N: usize, PairId, Topic, Pool, Order, PoolIndex, OrderIndex, K, Proto, Ctx>
    EventHandler<MempoolUpdate<TxViewMut>>
    for SpecializedHandler<
        PairUpdateHandler<N, PairId, Topic, Order, PoolIndex, Proto, Ctx>,
        OrderIndex,
        Pool,
        K,
        Ctx,
    >
where
    Proto: Copy + Send,
    Ctx: From<(Proto, EventContext<K>)> + Send,
    PairId: Copy + Hash + Eq + Send,
    Topic: Sink<(PairId, Channel<OrderUpdate<Order, Order>, LedgerCx>)> + Send + Unpin,
    Topic::Error: Debug,
    Pool: EntitySnapshot + Tradable<PairId = PairId> + Send,
    Order: SpecializedOrder<TPoolId = Pool::StableId>
        + TryFromLedger<TransactionOutput, Ctx>
        + Clone
        + Debug
        + Send,
    Order::TOrderId: From<OutputRef> + Display,
    OrderIndex: KvIndex<Order::TOrderId, Order> + Send,
    PoolIndex: TradableEntityIndex<Pool> + Send,
    K: Copy + Send,
{
    async fn try_handle(&mut self, ev: MempoolUpdate<TxViewMut>) -> Option<MempoolUpdate<TxViewMut>> {
        let mut updates: HashMap<PairId, Vec<Channel<OrderUpdate<Order, Order>, LedgerCx>>> = HashMap::new();
        let remainder = match ev {
            MempoolUpdate::TxAccepted(tx) => {
                match extract_atomic_transitions(
                    Arc::clone(&self.order_index),
                    self.general_handler.context_proto,
                    tx,
                )
                .await
                {
                    Ok((transitions, tx)) => {
                        trace!("{} entities found in accepted TX", transitions.len());
                        let pool_index = self.general_handler.index.lock().await;
                        let mut index = self.order_index.lock().await;
                        index.run_eviction();
                        for tr in transitions {
                            if let Some(pair) = pool_index.pair_of(&pool_ref_of(&tr)) {
                                index_atomic_transition(&mut index, &tr);
                                let upd = Channel::mempool(tr.into());
                                match updates.entry(pair) {
                                    Entry::Occupied(mut entry) => {
                                        entry.get_mut().push(upd);
                                    }
                                    Entry::Vacant(entry) => {
                                        entry.insert(vec![upd]);
                                    }
                                }
                            }
                        }
                        Some(MempoolUpdate::TxAccepted(tx))
                    }
                    Err(tx) => Some(MempoolUpdate::TxAccepted(tx)),
                }
            }
            MempoolUpdate::TxDropped(tx) => {
                match extract_atomic_transitions(
                    Arc::clone(&self.order_index),
                    self.general_handler.context_proto,
                    tx,
                )
                .await
                {
                    Ok((transitions, tx)) => {
                        trace!("{} entities found in dropped TX", transitions.len());
                        let pool_index = self.general_handler.index.lock().await;
                        let mut index = self.order_index.lock().await;
                        index.run_eviction();
                        for tr in transitions {
                            if let Some(pair) = pool_index.pair_of(&pool_ref_of(&tr)) {
                                let inverse_tr = tr.flip();
                                index_atomic_transition(&mut index, &inverse_tr);
                                let upd = Channel::mempool(inverse_tr.into());
                                match updates.entry(pair) {
                                    Entry::Occupied(mut entry) => {
                                        entry.get_mut().push(upd);
                                    }
                                    Entry::Vacant(entry) => {
                                        entry.insert(vec![upd]);
                                    }
                                }
                            }
                        }
                        Some(MempoolUpdate::TxDropped(tx))
                    }
                    Err(tx) => Some(MempoolUpdate::TxDropped(tx)),
                }
            }
        };
        for (pair, updates_by_pair) in updates {
            let num_updates = updates_by_pair.len();
            let topic = self.general_handler.topic.get_mut(pair);
            topic
                .batch_send_by_key(pair, updates_by_pair)
                .await
                .expect("Failed to submit updates");
            trace!("{} special mempool updates commited", num_updates);
        }
        remainder
    }
}

fn pool_ref_of<T: SpecializedOrder>(tr: &Either<T, T>) -> T::TPoolId {
    match tr {
        Either::Left(o) => o.get_pool_ref(),
        Either::Right(o) => o.get_pool_ref(),
    }
}

async fn extract_atomic_transitions<Order, Index, K, Proto, Ctx>(
    index: Arc<Mutex<Index>>,
    context_proto: Proto,
    mut tx: TxViewMut,
) -> Result<(Vec<Either<Order, Order>>, TxViewMut), TxViewMut>
where
    Proto: Copy,
    Ctx: From<(Proto, EventContext<K>)>,
    Order: SpecializedOrder + TryFromLedger<TransactionOutput, Ctx> + Clone,
    Order::TOrderId: From<OutputRef> + Display,
    Index: KvIndex<Order::TOrderId, Order>,
    K: Copy,
{
    let num_outputs = tx.outputs.len();
    if num_outputs == 0 {
        return Err(tx);
    }
    let mut consumed_orders = HashMap::<Order::TOrderId, Order>::new();
    let mut consumed_utxos = Vec::new();
    for i in &tx.inputs {
        let oref = OutputRef::from((i.transaction_id, i.index));
        consumed_utxos.push(oref);
        let state_id = Order::TOrderId::from(oref);
        let index = index.lock().await;
        if let Some(order) = index.get(&state_id) {
            let order_id = order.get_self_ref();
            trace!("Order {} eliminated by {}", order_id, tx.hash);
            consumed_orders.insert(order_id, order);
        }
    }
    let mut produced_orders = HashMap::<Order::TOrderId, Order>::new();
    let consumed_utxos = SmallVec::new(consumed_utxos.into_iter());
    let mut non_processed_outputs = VecDeque::new();
    while let Some((ix, o)) = tx.outputs.pop() {
        let o_ref = OutputRef::new(tx.hash, ix as u64);
        let event_context = EventContext {
            output_ref: o_ref,
            metadata: tx.metadata.clone(),
            consumed_utxos: consumed_utxos.into(),
            consumed_identifiers: Default::default(),
            produced_identifiers: Default::default(),
            added_payment_destinations: Default::default(),
            mints: tx.mints,
        };
        match Order::try_from_ledger(&o, &Ctx::from((context_proto, event_context))) {
            Some(order) => {
                let order_id = order.get_self_ref();
                trace!("Order {} created by {}", order_id, tx.hash);
                produced_orders.insert(order_id, order);
            }
            None => {
                non_processed_outputs.push_front((ix, o));
            }
        }
    }
    // Preserve non-processed outputs in original ordering.
    tx.outputs = non_processed_outputs.into();

    // Gather IDs of all recognized entities.
    let mut keys = HashSet::new();
    for k in consumed_orders.keys().chain(produced_orders.keys()) {
        keys.insert(*k);
    }

    // Match consumed versions with produced ones.
    let mut transitions = vec![];
    for k in keys.into_iter() {
        match (consumed_orders.remove(&k), produced_orders.remove(&k)) {
            (Some(consumed), _) => transitions.push(Either::Left(consumed)),
            (_, Some(produced)) => transitions.push(Either::Right(produced)),
            _ => {}
        };
    }

    if transitions.is_empty() {
        return Err(tx);
    }
    Ok((transitions, tx))
}

async fn extract_continuous_transitions<Entity, Index, Proto, Ctx>(
    index: Arc<Mutex<Index>>,
    context_proto: Proto,
    mut tx: TxViewMut,
) -> Result<(Vec<Ior<Entity, Entity>>, TxViewMut), TxViewMut>
where
    Proto: Copy,
    Ctx: From<(Proto, EventContext<Entity::StableId>)>,
    Entity: EntitySnapshot + Tradable + TryFromLedger<TransactionOutput, Ctx> + Clone,
    Entity::Version: From<OutputRef>,
    Index: TradableEntityIndex<Entity>,
{
    let num_outputs = tx.outputs.len();
    if num_outputs == 0 {
        return Err(tx);
    }
    let mut consumed_entities = HashMap::<Entity::StableId, Entity>::new();
    let mut consumed_utxos = Vec::new();
    for i in &tx.inputs {
        let oref = OutputRef::from((i.transaction_id, i.index));
        consumed_utxos.push(oref);
        let state_id = Entity::Version::from(oref);
        let mut index = index.lock().await;
        if index.exists(&state_id) {
            if let Some(entity) = index.get_state(&state_id) {
                let entity_id = entity.stable_id();
                trace!("Entity {} consumed by {}", entity_id, tx.hash);
                consumed_entities.insert(entity_id, entity);
            }
        }
    }
    let mut produced_entities = HashMap::<Entity::StableId, Entity>::new();
    let mut non_processed_outputs = VecDeque::new();
    let consumed_utxos = SmallVec::new(consumed_utxos.into_iter());
    let consumed_identifiers = SmallVec::new(consumed_entities.keys().cloned());
    let outbound_keys = tx.outputs.iter().filter_map(|(_, o)| match o.address() {
        Address::Base(BaseAddress {
            payment: Credential::PubKey { hash, .. },
            ..
        })
        | Address::Enterprise(EnterpriseAddress {
            payment: Credential::PubKey { hash, .. },
            ..
        }) => Some(hash),
        _ => None,
    });
    let added_destinations = AddedPaymentDestinations(SmallVec::new(outbound_keys.filter_map(|dst| {
        if !tx.signers.contains(dst) {
            Some(*dst)
        } else {
            None
        }
    })));
    while let Some((ix, o)) = tx.outputs.pop() {
        let o_ref = OutputRef::new(tx.hash, ix as u64);
        let produced_identifiers = SmallVec::new(produced_entities.keys().cloned());
        let event_context = EventContext {
            output_ref: o_ref,
            metadata: tx.metadata.clone(),
            consumed_utxos: consumed_utxos.into(),
            consumed_identifiers: consumed_identifiers.into(),
            produced_identifiers: produced_identifiers.into(),
            added_payment_destinations: added_destinations,
            mints: tx.mints,
        };
        match Entity::try_from_ledger(&o, &Ctx::from((context_proto, event_context))) {
            Some(entity) => {
                let entity_id = entity.stable_id();
                trace!("Entity {} created by {}", entity_id, tx.hash);
                produced_entities.insert(entity_id, entity);
            }
            None => {
                non_processed_outputs.push_front((ix, o));
            }
        }
    }
    // Preserve non-processed outputs in original ordering.
    tx.outputs = non_processed_outputs.into();

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
    Ok((transitions, tx))
}

fn pair_id_of<T: Tradable>(xa: &Ior<T, T>) -> T::PairId {
    match xa {
        Ior::Left(o) => o.pair_id(),
        Ior::Right(o) => o.pair_id(),
        Ior::Both(o, _) => o.pair_id(),
    }
}

#[async_trait]
impl<const N: usize, PairId, Topic, Entity, Index, Proto, Ctx> EventHandler<LedgerTxEvent<TxViewMut>>
    for PairUpdateHandler<N, PairId, Topic, Entity, Index, Proto, Ctx>
where
    Proto: Copy + Send,
    Ctx: From<(Proto, EventContext<Entity::StableId>)> + Send,
    PairId: Copy + Hash + Eq + Send,
    Topic: Sink<(PairId, Channel<Transition<Entity>, LedgerCx>)> + Unpin + Send,
    Topic::Error: Debug,
    Entity: EntitySnapshot
        + Tradable<PairId = PairId>
        + TryFromLedger<TransactionOutput, Ctx>
        + Clone
        + Debug
        + Send,
    Entity::Version: From<OutputRef>,
    Index: TradableEntityIndex<Entity> + Send,
{
    async fn try_handle(&mut self, ev: LedgerTxEvent<TxViewMut>) -> Option<LedgerTxEvent<TxViewMut>> {
        let mut updates: HashMap<PairId, Vec<Channel<Transition<Entity>, LedgerCx>>> = HashMap::new();
        let remainder = match ev {
            LedgerTxEvent::TxApplied {
                tx,
                slot,
                block_number,
                block_hash,
            } => {
                match extract_continuous_transitions(Arc::clone(&self.index), self.context_proto, tx).await {
                    Ok((transitions, tx)) => {
                        trace!("{} transitions found in applied TX", transitions.len());
                        let cx = LedgerCx::new(block_hash, slot);
                        let mut index = self.index.lock().await;
                        index.run_eviction();
                        for tr in transitions {
                            index_transition(&mut index, &tr);
                            let pair = pair_id_of(&tr);
                            let upd = Channel::ledger(Transition::Forward(tr), cx);
                            match updates.entry(pair) {
                                Entry::Occupied(mut entry) => {
                                    entry.get_mut().push(upd);
                                }
                                Entry::Vacant(entry) => {
                                    entry.insert(vec![upd]);
                                }
                            }
                        }
                        Some(LedgerTxEvent::TxApplied {
                            tx,
                            slot,
                            block_number,
                            block_hash,
                        })
                    }
                    Err(tx) => Some(LedgerTxEvent::TxApplied {
                        tx,
                        slot,
                        block_number,
                        block_hash,
                    }),
                }
            }
            LedgerTxEvent::TxUnapplied {
                tx,
                slot,
                block_number,
                block_hash,
            } => {
                match extract_continuous_transitions(Arc::clone(&self.index), self.context_proto, tx).await {
                    Ok((transitions, tx)) => {
                        trace!("{} entities found in unapplied TX", transitions.len());
                        let cx = LedgerCx::new(block_hash, slot);
                        let mut index = self.index.lock().await;
                        index.run_eviction();
                        for tr in transitions {
                            let inverse_tr = tr.swap();
                            index_transition(&mut index, &inverse_tr);
                            let pair = pair_id_of(&inverse_tr);
                            let upd = Channel::ledger(Transition::Backward(inverse_tr), cx);
                            match updates.entry(pair) {
                                Entry::Occupied(mut entry) => {
                                    entry.get_mut().push(upd);
                                }
                                Entry::Vacant(entry) => {
                                    entry.insert(vec![upd]);
                                }
                            }
                        }
                        Some(LedgerTxEvent::TxUnapplied {
                            tx,
                            slot,
                            block_number,
                            block_hash,
                        })
                    }
                    Err(tx) => Some(LedgerTxEvent::TxUnapplied {
                        tx,
                        slot,
                        block_number,
                        block_hash,
                    }),
                }
            }
        };
        for (pair, updates_by_pair) in updates {
            let num_updates = updates_by_pair.len();
            let topic = self.topic.get_mut(pair);
            topic
                .batch_send_by_key(pair, updates_by_pair)
                .await
                .expect("Failed to submit updates");
            trace!("{} updates commited", num_updates);
        }
        remainder
    }
}

#[async_trait]
impl<const N: usize, PairId, Topic, Entity, Index, Proto, Ctx> EventHandler<MempoolUpdate<TxViewMut>>
    for PairUpdateHandler<N, PairId, Topic, Entity, Index, Proto, Ctx>
where
    Proto: Copy + Send,
    Ctx: From<(Proto, EventContext<Entity::StableId>)> + Send,
    PairId: Copy + Hash + Eq + Send,
    Topic: Sink<(PairId, Channel<Transition<Entity>, LedgerCx>)> + Unpin + Send,
    Topic::Error: Debug,
    Entity: EntitySnapshot
        + Tradable<PairId = PairId>
        + TryFromLedger<TransactionOutput, Ctx>
        + Clone
        + Debug
        + Send,
    Entity::Version: From<OutputRef>,
    Index: TradableEntityIndex<Entity> + Send,
{
    async fn try_handle(&mut self, ev: MempoolUpdate<TxViewMut>) -> Option<MempoolUpdate<TxViewMut>> {
        let mut updates: HashMap<PairId, Vec<Channel<Transition<Entity>, LedgerCx>>> = HashMap::new();
        let remainder = match ev {
            MempoolUpdate::TxAccepted(tx) => {
                match extract_continuous_transitions(Arc::clone(&self.index), self.context_proto, tx).await {
                    Ok((transitions, tx)) => {
                        trace!("{} entities found in accepted TX", transitions.len());
                        let mut index = self.index.lock().await;
                        index.run_eviction();
                        for tr in transitions {
                            index_transition(&mut index, &tr);
                            let pair = pair_id_of(&tr);
                            let upd = Channel::mempool(Transition::Forward(tr));
                            match updates.entry(pair) {
                                Entry::Occupied(mut entry) => {
                                    entry.get_mut().push(upd);
                                }
                                Entry::Vacant(entry) => {
                                    entry.insert(vec![upd]);
                                }
                            }
                        }
                        Some(MempoolUpdate::TxAccepted(tx))
                    }
                    Err(tx) => Some(MempoolUpdate::TxAccepted(tx)),
                }
            }
            MempoolUpdate::TxDropped(tx) => {
                match extract_continuous_transitions(Arc::clone(&self.index), self.context_proto, tx).await {
                    Ok((transitions, tx)) => {
                        trace!("{} entities found in dropped TX", transitions.len());
                        let mut index = self.index.lock().await;
                        index.run_eviction();
                        for tr in transitions {
                            index_transition(&mut index, &tr);
                            let inverse_tr = tr.swap();
                            let pair = pair_id_of(&inverse_tr);
                            let upd = Channel::mempool(Transition::Backward(inverse_tr));
                            match updates.entry(pair) {
                                Entry::Occupied(mut entry) => {
                                    entry.get_mut().push(upd);
                                }
                                Entry::Vacant(entry) => {
                                    entry.insert(vec![upd]);
                                }
                            }
                        }
                        Some(MempoolUpdate::TxDropped(tx))
                    }
                    Err(tx) => Some(MempoolUpdate::TxDropped(tx)),
                }
            }
        };
        for (pair, updates_by_pair) in updates {
            let num_updates = updates_by_pair.len();
            let topic = self.topic.get_mut(pair);
            topic
                .batch_send_by_key(pair, updates_by_pair)
                .await
                .expect("Failed to submit updates");
            trace!("{} mempool updates commited", num_updates);
        }
        remainder
    }
}

fn index_atomic_transition<Index, T>(index: &mut MutexGuard<Index>, tr: &Either<T, T>)
where
    T: SpecializedOrder + Clone,
    Index: KvIndex<T::TOrderId, T>,
{
    match &tr {
        Either::Left(consumed) => {
            index.register_for_eviction(consumed.get_self_ref());
        }
        Either::Right(produced) => {
            index.put(produced.get_self_ref(), produced.clone());
        }
    }
}

fn index_transition<Index, T>(index: &mut MutexGuard<Index>, tr: &Ior<T, T>)
where
    T: EntitySnapshot + Tradable + Clone,
    Index: TradableEntityIndex<T>,
{
    match &tr {
        Ior::Left(consumed) => {
            index.register_for_eviction(consumed.version());
        }
        Ior::Right(produced) => {
            index.put_state(produced.clone());
        }
        Ior::Both(consumed, produced) => {
            index.register_for_eviction(consumed.version());
            index.put_state(produced.clone());
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fmt::{Debug, Formatter};
    use std::sync::Arc;
    use std::time::Duration;

    use cml_chain::address::{Address, RewardAddress};
    use cml_chain::certs::Credential;
    use cml_chain::transaction::{
        ConwayFormatTxOut, Transaction, TransactionBody, TransactionInput, TransactionOutput,
        TransactionWitnessSet,
    };
    use cml_crypto::{BlockHeaderHash, Ed25519KeyHash, ScriptHash};
    use futures::channel::mpsc;
    use futures::StreamExt;
    use tokio::sync::Mutex;

    use crate::event_sink::context::{HandlerContext, HandlerContextProto};
    use crate::event_sink::entity_index::InMemoryEntityIndex;
    use crate::event_sink::handler::{LedgerCx, PairUpdateHandler, TxViewMut};
    use crate::orders::adhoc::AdhocFeeStructure;
    use crate::orders::limit::LimitOrderValidation;
    use crate::validation_rules::ValidationRules;
    use algebra_core::monoid::Monoid;
    use cardano_chain_sync::data::LedgerTxEvent;
    use spectrum_cardano_lib::ex_units::ExUnits;
    use spectrum_cardano_lib::hash::hash_transaction_canonical;
    use spectrum_cardano_lib::transaction::TransactionOutputExtension;
    use spectrum_cardano_lib::OutputRef;
    use spectrum_offchain::data::ior::Ior;
    use spectrum_offchain::domain::event::{Channel, Confirmed, Transition};
    use spectrum_offchain::domain::{EntitySnapshot, Has, Stable, Tradable};
    use spectrum_offchain::event_sink::event_handler::EventHandler;
    use spectrum_offchain::ledger::TryFromLedger;
    use spectrum_offchain::partitioning::Partitioned;
    use spectrum_offchain_cardano::creds::OperatorCred;
    use spectrum_offchain_cardano::data::dao_request::{DAOContext, DAOV1ActionOrderValidation};
    use spectrum_offchain_cardano::data::deposit::DepositOrderValidation;
    use spectrum_offchain_cardano::data::pool::PoolValidation;
    use spectrum_offchain_cardano::data::redeem::RedeemOrderValidation;
    use spectrum_offchain_cardano::data::royalty_withdraw_request::RoyaltyWithdrawOrderValidation;
    use spectrum_offchain_cardano::deployment::{DeployedScriptInfo, ProtocolScriptHashes};

    #[derive(Clone, Eq, PartialEq)]
    struct TrivialEntity(OutputRef, u64);

    impl Debug for TrivialEntity {
        fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
            f.write_str(format!("TrivialEntity({}, {})", self.0, self.1).as_str())
        }
    }

    impl Tradable for TrivialEntity {
        type PairId = u8;
        fn pair_id(&self) -> Self::PairId {
            0
        }
    }

    impl Stable for TrivialEntity {
        type StableId = u8;
        fn stable_id(&self) -> Self::StableId {
            0
        }
        fn is_quasi_permanent(&self) -> bool {
            false
        }
    }

    impl EntitySnapshot for TrivialEntity {
        type Version = OutputRef;
        fn version(&self) -> Self::Version {
            self.0
        }
    }

    impl<C> TryFromLedger<TransactionOutput, C> for TrivialEntity
    where
        C: Has<OutputRef>,
    {
        fn try_from_ledger(repr: &TransactionOutput, ctx: &C) -> Option<Self> {
            Some(TrivialEntity(ctx.select::<OutputRef>(), repr.value().coin))
        }
    }

    #[tokio::test]
    async fn apply_unapply_transaction() {
        let block_hash = BlockHeaderHash::from([0u8; 32]);
        let block_number = 1;
        let slot = 1;
        let (amt_1, amt_2) = (1000u64, 98000u64);
        let fee = 1000;
        let utxo_1 = TransactionOutput::new_conway_format_tx_out(ConwayFormatTxOut::new(
            Address::Reward(RewardAddress::new(
                0,
                Credential::PubKey {
                    hash: Ed25519KeyHash::from([0u8; 28]),
                    len_encoding: Default::default(),
                    tag_encoding: None,
                    hash_encoding: Default::default(),
                },
            )),
            amt_1.into(),
        ));
        let utxo_2 = TransactionOutput::new_conway_format_tx_out(ConwayFormatTxOut::new(
            Address::Reward(RewardAddress::new(
                0,
                Credential::PubKey {
                    hash: Ed25519KeyHash::from([1u8; 28]),
                    len_encoding: Default::default(),
                    tag_encoding: None,
                    hash_encoding: Default::default(),
                },
            )),
            amt_2.into(),
        ));
        let tx_1 = Transaction::new(
            TransactionBody::new(vec![].into(), vec![utxo_1], fee),
            TransactionWitnessSet::new(),
            true,
            None,
        );
        let tx_1_hash = hash_transaction_canonical(&tx_1.body);
        let tx_2 = Transaction::new(
            TransactionBody::new(
                vec![TransactionInput::new(tx_1_hash, 0)].into(),
                vec![utxo_2].into(),
                1000,
            ),
            TransactionWitnessSet::new(),
            true,
            None,
        );
        let tx_2_hash = hash_transaction_canonical(&tx_2.body);
        let entity_eviction_delay = Duration::from_secs(60 * 5);
        let index = Arc::new(Mutex::new(InMemoryEntityIndex::new(entity_eviction_delay)));
        let (snd, mut recv) = mpsc::channel::<(u8, Channel<Transition<TrivialEntity>, LedgerCx>)>(100);
        let ex_cred = OperatorCred(Ed25519KeyHash::from([0u8; 28]));
        let context = HandlerContextProto {
            validation_rules: ValidationRules {
                limit_order: LimitOrderValidation {
                    min_cost_per_ex_step: 1000,
                    min_fee_lovelace: 1000,
                },
                deposit_order: DepositOrderValidation {
                    min_collateral_ada: 1000,
                },
                redeem_order: RedeemOrderValidation {
                    min_collateral_ada: 1000,
                },
                pool: PoolValidation {
                    min_n2t_lovelace: 1000,
                    min_t2t_lovelace: 1000,
                },
                royalty_withdraw: RoyaltyWithdrawOrderValidation {
                    min_ada_in_royalty_output: 0,
                },
                dao_action: DAOV1ActionOrderValidation {
                    min_collateral_ada: 0,
                },
            },
            executor_cred: ex_cred,
            scripts: ProtocolScriptHashes {
                limit_order_witness: DeployedScriptInfo {
                    script_hash: ScriptHash::from([0u8; 28]),
                    marginal_cost: ExUnits::empty(),
                },
                limit_order: DeployedScriptInfo {
                    script_hash: ScriptHash::from([0u8; 28]),
                    marginal_cost: ExUnits::empty(),
                },
                instant_order_witness: DeployedScriptInfo {
                    script_hash: ScriptHash::from([0u8; 28]),
                    marginal_cost: ExUnits::empty(),
                },
                instant_order: DeployedScriptInfo {
                    script_hash: ScriptHash::from([0u8; 28]),
                    marginal_cost: ExUnits::empty(),
                },
                grid_order_native: DeployedScriptInfo {
                    script_hash: ScriptHash::from([0u8; 28]),
                    marginal_cost: ExUnits::empty(),
                },
                const_fn_pool_v1: DeployedScriptInfo {
                    script_hash: ScriptHash::from([0u8; 28]),
                    marginal_cost: ExUnits::empty(),
                },
                const_fn_pool_v2: DeployedScriptInfo {
                    script_hash: ScriptHash::from([0u8; 28]),
                    marginal_cost: ExUnits::empty(),
                },
                const_fn_pool_fee_switch: DeployedScriptInfo {
                    script_hash: ScriptHash::from([0u8; 28]),
                    marginal_cost: ExUnits::empty(),
                },
                const_fn_pool_fee_switch_bidir_fee: DeployedScriptInfo {
                    script_hash: ScriptHash::from([0u8; 28]),
                    marginal_cost: ExUnits::empty(),
                },
                const_fn_fee_switch_pool_swap: DeployedScriptInfo {
                    script_hash: ScriptHash::from([0u8; 28]),
                    marginal_cost: ExUnits::empty(),
                },
                const_fn_fee_switch_pool_deposit: DeployedScriptInfo {
                    script_hash: ScriptHash::from([0u8; 28]),
                    marginal_cost: ExUnits::empty(),
                },
                const_fn_fee_switch_pool_redeem: DeployedScriptInfo {
                    script_hash: ScriptHash::from([0u8; 28]),
                    marginal_cost: ExUnits::empty(),
                },
                balance_fn_pool_v1: DeployedScriptInfo {
                    script_hash: ScriptHash::from([0u8; 28]),
                    marginal_cost: ExUnits::empty(),
                },
                balance_fn_pool_deposit: DeployedScriptInfo {
                    script_hash: ScriptHash::from([0u8; 28]),
                    marginal_cost: ExUnits::empty(),
                },
                balance_fn_pool_redeem: DeployedScriptInfo {
                    script_hash: ScriptHash::from([0u8; 28]),
                    marginal_cost: ExUnits::empty(),
                },
                const_fn_pool_deposit: DeployedScriptInfo {
                    script_hash: ScriptHash::from([0u8; 28]),
                    marginal_cost: ExUnits::empty(),
                },
                const_fn_pool_redeem: DeployedScriptInfo {
                    script_hash: ScriptHash::from([0u8; 28]),
                    marginal_cost: ExUnits::empty(),
                },
                const_fn_pool_swap: DeployedScriptInfo {
                    script_hash: ScriptHash::from([0u8; 28]),
                    marginal_cost: ExUnits::empty(),
                },
                stable_fn_pool_t2t: DeployedScriptInfo {
                    script_hash: ScriptHash::from([0u8; 28]),
                    marginal_cost: ExUnits::empty(),
                },
                stable_fn_pool_t2t_deposit: DeployedScriptInfo {
                    script_hash: ScriptHash::from([0u8; 28]),
                    marginal_cost: ExUnits::empty(),
                },
                stable_fn_pool_t2t_redeem: DeployedScriptInfo {
                    script_hash: ScriptHash::from([0u8; 28]),
                    marginal_cost: ExUnits::empty(),
                },
                royalty_pool_v1: DeployedScriptInfo {
                    script_hash: ScriptHash::from([0u8; 28]),
                    marginal_cost: ExUnits::empty(),
                },
                royalty_pool_v1_ledger_fixed: DeployedScriptInfo {
                    script_hash: ScriptHash::from([0u8; 28]),
                    marginal_cost: ExUnits::empty(),
                },
                royalty_pool_v2: DeployedScriptInfo {
                    script_hash: ScriptHash::from([0u8; 28]),
                    marginal_cost: ExUnits::empty(),
                },
                royalty_pool_deposit: DeployedScriptInfo {
                    script_hash: ScriptHash::from([0u8; 28]),
                    marginal_cost: ExUnits::empty(),
                },
                royalty_pool_deposit_v2: DeployedScriptInfo {
                    script_hash: ScriptHash::from([0u8; 28]),
                    marginal_cost: ExUnits::empty(),
                },
                royalty_pool_redeem: DeployedScriptInfo {
                    script_hash: ScriptHash::from([0u8; 28]),
                    marginal_cost: ExUnits::empty(),
                },
                royalty_pool_redeem_v2: DeployedScriptInfo {
                    script_hash: ScriptHash::from([0u8; 28]),
                    marginal_cost: ExUnits::empty(),
                },
                royalty_pool_withdraw_request: DeployedScriptInfo {
                    script_hash: ScriptHash::from([0u8; 28]),
                    marginal_cost: ExUnits::empty(),
                },
                royalty_pool_v2_withdraw_request: DeployedScriptInfo {
                    script_hash: ScriptHash::from([0u8; 28]),
                    marginal_cost: ExUnits::empty(),
                },
                royalty_pool_dao_request: DeployedScriptInfo {
                    script_hash: ScriptHash::from([0u8; 28]),
                    marginal_cost: ExUnits::empty(),
                },
                royalty_pool_dao: DeployedScriptInfo {
                    script_hash: ScriptHash::from([0u8; 28]),
                    marginal_cost: ExUnits::empty(),
                },
                balance_fn_pool_v2: DeployedScriptInfo {
                    script_hash: ScriptHash::from([0u8; 28]),
                    marginal_cost: ExUnits::empty(),
                },
                const_fn_pool_fee_switch_v2: DeployedScriptInfo {
                    script_hash: ScriptHash::from([0u8; 28]),
                    marginal_cost: ExUnits::empty(),
                },

                royalty_pool_withdraw: DeployedScriptInfo {
                    script_hash: ScriptHash::from([0u8; 28]),
                    marginal_cost: ExUnits::empty(),
                },
                royalty_pool_dao_v2: DeployedScriptInfo {
                    script_hash: ScriptHash::from([0u8; 28]),
                    marginal_cost: ExUnits::empty(),
                },
            },
            adhoc_fee_structure: AdhocFeeStructure::empty(),
            dao_context: DAOContext {
                public_keys: Default::default(),
                signature_threshold: 0,
                execution_fee: 0,
            },
        };
        let mut handler: PairUpdateHandler<
            1,
            u8,
            mpsc::Sender<(u8, Channel<Transition<TrivialEntity>, LedgerCx>)>,
            TrivialEntity,
            InMemoryEntityIndex<TrivialEntity>,
            HandlerContextProto,
            HandlerContext<u8>,
        > = PairUpdateHandler::new(Partitioned::new([snd]), index, context);
        // Handle tx application
        EventHandler::<LedgerTxEvent<TxViewMut>>::try_handle(
            &mut handler,
            LedgerTxEvent::TxApplied {
                tx: tx_1.into(),
                slot,
                block_number,
                block_hash,
            },
        )
        .await;
        let (_, Channel::Ledger(Confirmed(Transition::Forward(Ior::Right(e1))), _)) =
            recv.next().await.expect("Must result in new event")
        else {
            panic!("Must be a transition")
        };
        EventHandler::<LedgerTxEvent<TxViewMut>>::try_handle(
            &mut handler,
            LedgerTxEvent::TxApplied {
                tx: tx_2.clone().into(),
                slot,
                block_number,
                block_hash,
            },
        )
        .await;
        let (_, Channel::Ledger(Confirmed(Transition::Forward(Ior::Both(e1_reversed, e2))), _)) =
            recv.next().await.expect("Must result in new event")
        else {
            panic!("Must be a transition")
        };
        assert_eq!(e1_reversed, e1);
        EventHandler::<LedgerTxEvent<TxViewMut>>::try_handle(
            &mut handler,
            LedgerTxEvent::TxUnapplied {
                tx: tx_2.into(),
                slot,
                block_number,
                block_hash,
            },
        )
        .await;
        let (_, Channel::Ledger(Confirmed(Transition::Backward(Ior::Both(e2_reversed, e1_revived))), _)) =
            recv.next().await.expect("Must result in new event")
        else {
            panic!("Must be a transition")
        };
        assert_eq!(e2_reversed, e2);
        assert_eq!(e1_revived, e1);
    }
}
