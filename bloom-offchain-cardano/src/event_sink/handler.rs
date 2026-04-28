use std::collections::hash_map::Entry;
use std::collections::{HashMap, HashSet, VecDeque};
use std::fmt::{Debug, Display};
use std::hash::Hash;
use std::marker::PhantomData;
use std::sync::Arc;

use crate::event_sink::context::{EventContext, HandlerContextProto};
use crate::event_sink::entity_index::TradableEntityIndex;
use crate::event_sink::order_index::KvIndex;
use crate::event_sink::tx_view::TxViewMut;
use crate::graduation::SnekQuadraticPoolIdentity;
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
use spectrum_cardano_lib::{OutputRef, Token};
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

#[derive(Copy, Clone)]
enum GraduationAction {
    Apply,
    Rollback,
}

trait GraduationTracking {
    fn rollback_graduation(&self, tx_hash: cml_crypto::TransactionHash);
    fn consumed_snek_refs(&self, consumed_utxos: &[OutputRef]) -> Vec<(OutputRef, Token)>;
    fn observe_snek_output(
        &self,
        output_ref: OutputRef,
        output: &TransactionOutput,
    ) -> Option<(OutputRef, Token)>;
    fn journal_graduated_splash_pools(
        &self,
        tx_hash: cml_crypto::TransactionHash,
        consumed_snek_refs: Vec<(OutputRef, Token)>,
        produced_snek_refs: Vec<(OutputRef, Token)>,
        graduated_splash_ids: Vec<Token>,
    );
}

trait MaybeGraduatedSplashId {
    fn maybe_graduated_splash_id(&self) -> Option<Token>;
}

impl MaybeGraduatedSplashId for Token {
    fn maybe_graduated_splash_id(&self) -> Option<Token> {
        Some(*self)
    }
}

impl MaybeGraduatedSplashId for u8 {
    fn maybe_graduated_splash_id(&self) -> Option<Token> {
        None
    }
}

impl GraduationTracking for HandlerContextProto {
    fn rollback_graduation(&self, tx_hash: cml_crypto::TransactionHash) {
        self.graduated_pool_store
            .rollback_tx(tx_hash, &self.snek_pool_input_tracker);
    }

    fn consumed_snek_refs(&self, consumed_utxos: &[OutputRef]) -> Vec<(OutputRef, Token)> {
        consumed_utxos
            .iter()
            .filter_map(|oref| {
                self.snek_pool_input_tracker
                    .contains(*oref)
                    .map(|pool_id| (*oref, pool_id))
            })
            .collect()
    }

    fn observe_snek_output(
        &self,
        output_ref: OutputRef,
        output: &TransactionOutput,
    ) -> Option<(OutputRef, Token)> {
        let pool_id = SnekQuadraticPoolIdentity::try_from_ledger(output)?.pool_id;
        self.snek_pool_input_tracker.insert(output_ref, pool_id);
        Some((output_ref, pool_id))
    }

    fn journal_graduated_splash_pools(
        &self,
        tx_hash: cml_crypto::TransactionHash,
        consumed_snek_refs: Vec<(OutputRef, Token)>,
        produced_snek_refs: Vec<(OutputRef, Token)>,
        graduated_splash_ids: Vec<Token>,
    ) {
        self.graduated_pool_store.apply_observation(
            tx_hash,
            &self.snek_pool_input_tracker,
            consumed_snek_refs,
            produced_snek_refs,
            graduated_splash_ids,
        );
    }
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
    Proto: Clone + GraduationTracking + Send,
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
                    self.general_handler.context_proto.clone(),
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
                    self.general_handler.context_proto.clone(),
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
    Proto: Clone + Send,
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
                    self.general_handler.context_proto.clone(),
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
                    self.general_handler.context_proto.clone(),
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
    Proto: Clone,
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
        match Order::try_from_ledger(&o, &Ctx::from((context_proto.clone(), event_context))) {
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
    graduation_action: GraduationAction,
) -> Result<(Vec<Ior<Entity, Entity>>, TxViewMut), TxViewMut>
where
    Proto: Clone + GraduationTracking,
    Ctx: From<(Proto, EventContext<Entity::StableId>)>,
    Entity: EntitySnapshot + Tradable + TryFromLedger<TransactionOutput, Ctx> + Clone,
    Entity::StableId: MaybeGraduatedSplashId,
    Entity::Version: From<OutputRef>,
    Index: TradableEntityIndex<Entity>,
{
    let num_outputs = tx.outputs.len();
    if num_outputs == 0 {
        return Err(tx);
    }
    if matches!(graduation_action, GraduationAction::Rollback) {
        context_proto.rollback_graduation(tx.hash);
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
    let consumed_snek_refs = context_proto.consumed_snek_refs(&consumed_utxos);
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
    let mut produced_snek_refs = Vec::new();
    while let Some((ix, o)) = tx.outputs.pop() {
        let o_ref = OutputRef::new(tx.hash, ix as u64);
        if matches!(graduation_action, GraduationAction::Apply) {
            if let Some(produced_snek_ref) = context_proto.observe_snek_output(o_ref, &o) {
                produced_snek_refs.push(produced_snek_ref);
            }
        }
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
        match Entity::try_from_ledger(&o, &Ctx::from((context_proto.clone(), event_context))) {
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
    if matches!(graduation_action, GraduationAction::Apply) && !consumed_snek_refs.is_empty() {
        let graduated_splash_ids = produced_entities
            .keys()
            .filter_map(MaybeGraduatedSplashId::maybe_graduated_splash_id)
            .collect::<Vec<_>>();
        if !graduated_splash_ids.is_empty() {
            context_proto.journal_graduated_splash_pools(
                tx.hash,
                consumed_snek_refs,
                produced_snek_refs,
                graduated_splash_ids,
            );
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
    Proto: Clone + GraduationTracking + Send,
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
    Entity::StableId: MaybeGraduatedSplashId,
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
                match extract_continuous_transitions(
                    Arc::clone(&self.index),
                    self.context_proto.clone(),
                    tx,
                    GraduationAction::Apply,
                )
                .await
                {
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
                match extract_continuous_transitions(
                    Arc::clone(&self.index),
                    self.context_proto.clone(),
                    tx,
                    GraduationAction::Rollback,
                )
                .await
                {
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
    Proto: Clone + GraduationTracking + Send,
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
    Entity::StableId: MaybeGraduatedSplashId,
    Index: TradableEntityIndex<Entity> + Send,
{
    async fn try_handle(&mut self, ev: MempoolUpdate<TxViewMut>) -> Option<MempoolUpdate<TxViewMut>> {
        let mut updates: HashMap<PairId, Vec<Channel<Transition<Entity>, LedgerCx>>> = HashMap::new();
        let remainder = match ev {
            MempoolUpdate::TxAccepted(tx) => {
                match extract_continuous_transitions(
                    Arc::clone(&self.index),
                    self.context_proto.clone(),
                    tx,
                    GraduationAction::Apply,
                )
                .await
                {
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
                match extract_continuous_transitions(
                    Arc::clone(&self.index),
                    self.context_proto.clone(),
                    tx,
                    GraduationAction::Rollback,
                )
                .await
                {
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
    use cml_core::serialization::Deserialize;
    use cml_crypto::{BlockHeaderHash, Ed25519KeyHash, ScriptHash, TransactionHash};
    use futures::channel::mpsc;
    use futures::StreamExt;
    use tokio::sync::Mutex;

    use crate::event_sink::context::{HandlerContext, HandlerContextProto};
    use crate::event_sink::entity_index::InMemoryEntityIndex;
    use crate::event_sink::handler::{LedgerCx, PairUpdateHandler, TxViewMut};
    use crate::graduation::{GraduatedPoolFeeConfig, SnekQuadraticPoolIdentity};
    use crate::orders::adhoc::AdhocFeeStructure;
    use crate::orders::limit::LimitOrderValidation;
    use crate::pools::classified::ClassifiedPool;
    use crate::validation_rules::ValidationRules;
    use algebra_core::monoid::Monoid;
    use bloom_offchain::execution_engine::bundled::Bundled;
    use cardano_chain_sync::data::LedgerTxEvent;
    use spectrum_cardano_lib::ex_units::ExUnits;
    use spectrum_cardano_lib::hash::hash_transaction_canonical;
    use spectrum_cardano_lib::transaction::TransactionOutputExtension;
    use spectrum_cardano_lib::{OutputRef, Token};
    use spectrum_offchain::data::ior::Ior;
    use spectrum_offchain::domain::event::{Channel, Confirmed, Transition};
    use spectrum_offchain::domain::{Baked, EntitySnapshot, Has, Stable, Tradable};
    use spectrum_offchain::event_sink::event_handler::EventHandler;
    use spectrum_offchain::ledger::TryFromLedger;
    use spectrum_offchain::partitioning::Partitioned;
    use spectrum_offchain_cardano::creds::OperatorCred;
    use spectrum_offchain_cardano::data::dao_request::{DAOContext, DAOV1ActionOrderValidation};
    use spectrum_offchain_cardano::data::deposit::DepositOrderValidation;
    use spectrum_offchain_cardano::data::pool::PoolValidation;
    use spectrum_offchain_cardano::data::redeem::RedeemOrderValidation;
    use spectrum_offchain_cardano::data::royalty_withdraw_request::RoyaltyWithdrawOrderValidation;
    use spectrum_offchain_cardano::deployment::{
        DeployedScriptInfo, DeployedValidators, ProtocolScriptHashes,
    };

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
                royalty_pool_v2_dao_request: DeployedScriptInfo {
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
            graduated_pool_fee_config: Default::default(),
            graduated_pool_store: Default::default(),
            snek_pool_input_tracker: Default::default(),
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

    #[tokio::test]
    async fn real_graduation_tx_marks_produced_splash_pool() {
        let graduation_tx = Transaction::from_cbor_bytes(&hex::decode(GRADUATION_TX_CBOR).unwrap()).unwrap();
        let snek_pool_tx = Transaction::from_cbor_bytes(&hex::decode(SNEK_POOL_TX_CBOR).unwrap()).unwrap();
        let graduation_tx_hash = TransactionHash::from_hex(GRADUATION_TX_HASH).unwrap();
        let snek_pool_tx_hash = TransactionHash::from_hex(SNEK_POOL_TX_HASH).unwrap();
        let consumed_snek_ref = OutputRef::new(snek_pool_tx_hash, 1);
        let snek_pool_output = snek_pool_tx.body.outputs.get(1).unwrap();
        let snek_pool_id = SnekQuadraticPoolIdentity::try_from_ledger(snek_pool_output)
            .expect("consumed output must parse as Snek quadratic pool")
            .pool_id;

        type PoolEntity = Bundled<Baked<ClassifiedPool, OutputRef>, TransactionOutput>;
        type PairId = <PoolEntity as Tradable>::PairId;
        let entity_eviction_delay = Duration::from_secs(60 * 5);
        let index = Arc::new(Mutex::new(InMemoryEntityIndex::new(entity_eviction_delay)));
        let (snd, mut recv) = mpsc::channel::<(PairId, Channel<Transition<PoolEntity>, LedgerCx>)>(100);
        let context = mainnet_handler_context();
        context
            .snek_pool_input_tracker
            .insert(consumed_snek_ref, snek_pool_id);

        let mut handler: PairUpdateHandler<
            1,
            PairId,
            mpsc::Sender<(PairId, Channel<Transition<PoolEntity>, LedgerCx>)>,
            PoolEntity,
            InMemoryEntityIndex<PoolEntity>,
            HandlerContextProto,
            HandlerContext<Token>,
        > = PairUpdateHandler::new(Partitioned::new([snd]), index, context.clone());

        let block_hash = BlockHeaderHash::from([0u8; 32]);
        EventHandler::<LedgerTxEvent<TxViewMut>>::try_handle(
            &mut handler,
            LedgerTxEvent::TxApplied {
                tx: graduation_tx.into(),
                slot: 146451851,
                block_number: 11408210,
                block_hash,
            },
        )
        .await;

        let (_, Channel::Ledger(Confirmed(Transition::Forward(Ior::Right(pool))), _)) = recv
            .next()
            .await
            .expect("graduation tx must produce a Splash pool event")
        else {
            panic!("graduation tx must produce a new Splash pool")
        };

        assert_eq!(pool.version(), OutputRef::new(graduation_tx_hash, 0));
        assert_eq!(
            pool.stable_id(),
            Token::from_string_unsafe(
                "d8eb52caf3289a2880288b23141ce3d2a7025dcf76f26fd5659add06.de3498de00239a8372e540be094d1c17be04e35c5516094d04c572fcc287f391"
            )
        );
        assert!(context.graduated_pool_store.contains(pool.stable_id()));
        assert_eq!(context.snek_pool_input_tracker.contains(consumed_snek_ref), None);
    }

    fn mainnet_handler_context() -> HandlerContextProto {
        let deployment: DeployedValidators = serde_json::from_str(include_str!(
            "../../../bloom-cardano-agent/resources/mainnet.deployment.json"
        ))
        .unwrap();
        let validation_rules: ValidationRules = serde_json::from_str(include_str!(
            "../../../bloom-cardano-agent/resources/validation-rules.json.template"
        ))
        .unwrap();
        HandlerContextProto {
            validation_rules,
            executor_cred: OperatorCred(Ed25519KeyHash::from([0u8; 28])),
            scripts: ProtocolScriptHashes::from(&deployment),
            adhoc_fee_structure: AdhocFeeStructure::empty(),
            dao_context: DAOContext {
                public_keys: Default::default(),
                signature_threshold: 0,
                execution_fee: 0,
            },
            graduated_pool_fee_config: GraduatedPoolFeeConfig::enabled(1),
            graduated_pool_store: Default::default(),
            snek_pool_input_tracker: Default::default(),
        }
    }

    const GRADUATION_TX_HASH: &str = "f5029566d7e765c52909b23b1624928b05b4ec3e9cc3dcec04f32231c5f507b3";
    const SNEK_POOL_TX_HASH: &str = "f759b0da790f3dbf7c3c9767e9a39a0a87652ad17e459f16c3e3f4752c078c9d";
    const GRADUATION_TX_CBOR: &str = "84ab00d9010281825820f759b0da790f3dbf7c3c9767e9a39a0a87652ad17e459f16c3e3f4752c078c9d010182a300583931cb684a69e78907a9796b21fc150a758af5f2805e5ed5d5a8ce9f76f1b2f6abf60ccde92eae1a2f4fdf65f2eaf6208d872c6f0e597cc10b0701821b000000065ae65b94a3581cf71b4cf652d8edb33a57928b8b8a546a3c954b7ba24db5583ac79b34a14d534f4e474d41524b45544341501a10647224581cd8eb52caf3289a2880288b23141ce3d2a7025dcf76f26fd5659add06a15820de3498de00239a8372e540be094d1c17be04e35c5516094d04c572fcc287f39101581c6e917b8b965078a39804a6313e5be73535612421acd70aa83f0ec200a15820551dc3ea3f3cb1af3a3066a71370eb2ef25ac0cb0dfa416360eb950c7ab70e9d1b7fffffff5cb1c043028201d818590138d8799fd8799f581cd8eb52caf3289a2880288b23141ce3d2a7025dcf76f26fd5659add065820de3498de00239a8372e540be094d1c17be04e35c5516094d04c572fcc287f391ffd8799f4040ffd8799f581cf71b4cf652d8edb33a57928b8b8a546a3c954b7ba24db5583ac79b344d534f4e474d41524b4554434150ffd8799f581c6e917b8b965078a39804a6313e5be73535612421acd70aa83f0ec2005820551dc3ea3f3cb1af3a3066a71370eb2ef25ac0cb0dfa416360eb950c7ab70e9dff1a0001831c18321832000000009fd8799fd87a9f581c66e711a4bf9ddf46ff239143870b6893055a4fd4dea9f99fed6665cdffffff581c75c4570eb625ae881b32a34c52b159f6f3f3f2c7aaabf5bac4688133582072c68f905716a5f59a0ee2552ab68559f42287d335396d8f430da98e96c5009c00ff82581d61edbf33f5d6e083970648e39175c49ec1c093df76b6e6a0f1473e47761a0c7de81a021a0006706605a1581df130c1003aa7dec834e0d0a78db547ba8840e58060725dbfae352f0d6400075820d666cd3bef29084c3f60f5f099b86728411b6e5be0d2b347a52016141149cbba09a3581cd8eb52caf3289a2880288b23141ce3d2a7025dcf76f26fd5659add06a15820de3498de00239a8372e540be094d1c17be04e35c5516094d04c572fcc287f39101581c6e917b8b965078a39804a6313e5be73535612421acd70aa83f0ec200a15820551dc3ea3f3cb1af3a3066a71370eb2ef25ac0cb0dfa416360eb950c7ab70e9d1b7fffffff5cb1c043581c63f947b8d9535bc4e4ce6919e3dc056547e8d30ada12f29aa5f826b8a1582073c8bd90386c5dda23939ed10975283e6ddb9ddbd66bdb9bfb81e99259c8df3c200b5820bc110208a0d2540f6505c2785dd203853639249296780af5fd8f6f89d4f7767c0dd9010281825820b16ed2fdf1c9c7c8ff1b269057dd09db4655567da3b8474a83ad5fcce8921a8f011082581d61edbf33f5d6e083970648e39175c49ec1c093df76b6e6a0f1473e47761a020c73ad111a004c4b4012d90102858258204e847c2b0e482bc0b6377712363ab807068b8cae5442914c1d411d2bc798cee7038258204e847c2b0e482bc0b6377712363ab807068b8cae5442914c1d411d2bc798cee7048258204e847c2b0e482bc0b6377712363ab807068b8cae5442914c1d411d2bc798cee705825820b16ed2fdf1c9c7c8ff1b269057dd09db4655567da3b8474a83ad5fcce8921a8f00825820c4a540ac2e06c217dd4fb3f39ca3863da394ba134677dafa9b98830ca71d584d03a200d9010281825820b2f97417886f87a990b7f2a6c78c8210d6bcf8c93498d6a5c54f7026a9948d7558404f669dfb98900e8f0068d741aba55085f978b43cb0952002f1331b05c137ff04b5960f05daf7c696c3a8b92ca550ab669b9b198f95b2e99118dfa34ecf2df2090585840000d8799f0000d87b80ff821a00028e4f1a0301baca840100d8799fd8799f5820f759b0da790f3dbf7c3c9767e9a39a0a87652ad17e459f16c3e3f4752c078c9dff01ff8219c0ab1a00d4db7a840101d8799fd8799fd8799f5820f759b0da790f3dbf7c3c9767e9a39a0a87652ad17e459f16c3e3f4752c078c9dff01ff1b7fffffff5cb1c043ff821a000143131a01bc762e840102d8799fd8799f5820f759b0da790f3dbf7c3c9767e9a39a0a87652ad17e459f16c3e3f4752c078c9dff01ff821a000131cd1a01a7fb5384030000821a000829271a0a9e5233f5a1182a82582001e9000f529bb62d73ac4a5f7be031e47994e46698530bd1e807590040382aba5819f9dd61bceb9ddd14ffe0331a5708f842af4abdbdfb84d074b0";
    const SNEK_POOL_TX_CBOR: &str = "84a800828258200367c55973c4b158e2d834c53123869597eff89c006362e2da1b9d7ab092444d00825820b1e7b0b37bf06110b2bab376cd979d10cb02919eda302798b46e578bcc160a14020183a200583901355800d417486225ece1530b06c66c81e2963db332606657d7a894dbbe69325ef57e74875e56ab5405c3e3d051b0efed2f5398c110cfa3c901821a02e02859a1581cf71b4cf652d8edb33a57928b8b8a546a3c954b7ba24db5583ac79b34a14d534f4e474d41524b45544341501a00359d1ca300583931905ab869961b094f1b8197278cfe15b45cbe49fa8f32c6b014f85a2db2f6abf60ccde92eae1a2f4fdf65f2eaf6208d872c6f0e597cc10b0701821b00000006676ab414a2581c63f947b8d9535bc4e4ce6919e3dc056547e8d30ada12f29aa5f826b8a1582073c8bd90386c5dda23939ed10975283e6ddb9ddbd66bdb9bfb81e99259c8df3c01581cf71b4cf652d8edb33a57928b8b8a546a3c954b7ba24db5583ac79b34a14d534f4e474d41524b45544341501a10647224028201d81858ebd87989d87982581c63f947b8d9535bc4e4ce6919e3dc056547e8d30ada12f29aa5f826b8582073c8bd90386c5dda23939ed10975283e6ddb9ddbd66bdb9bfb81e99259c8df3cd879824040d87982581cf71b4cf652d8edb33a57928b8b8a546a3c954b7ba24db5583ac79b344d534f4e474d41524b45544341501b000000293daab8471a00694ff2581cedbf33f5d6e083970648e39175c49ec1c093df76b6e6a0f1473e47761b00000006676ab414581c8807fbe6e36b1c35ad6f36f0993e2fc67ab6f2db06041cfa3a53c04a581c30c1003aa7dec834e0d0a78db547ba8840e58060725dbfae352f0d648258390122aea2da15e494e01767145d48bda16b6d437f1c449823a044193daf299a82ef56311aa10adf04c0072d4870eb9f4d5ff315132434841b741a003ffae7021a0005d35105a1581df196f5c1bee23481335ff4aece32fe1dfa1aa40a944a66d2d6edc9a9a5000b58203a98f88216e5025db28f06e90c56b3def9f35e3ac2beef37aef7f09ace7a60c00d8182582006f24741c2cbc3f59096046de95fcd5d90a77e5378d95fd60f2ec261aa5c7981000e81581cedbf33f5d6e083970648e39175c49ec1c093df76b6e6a0f1473e47761283825820b91eda29d145ab6c0bc0d6b7093cb24b131440b7b015033205476f39c690a51f00825820c4a540ac2e06c217dd4fb3f39ca3863da394ba134677dafa9b98830ca71d584d03825820b91eda29d145ab6c0bc0d6b7093cb24b131440b7b015033205476f39c690a51f01a2008182582014585da9857b30cf57edccac19d3012b602bb5650c7726a5f1504d9614d2052258405bad8ff43f1d3e9330270fc9483c392bd98df4c538954f2825279ae81a7059bcd093dfd991b06b495ca3a48915271f276acd065ac47532975fedcb43608e600d0583840000d87a80821a000186a01a01c9c380840001d879830101d87980821a000864701a0d1cef0084030080821a000668a01a09896800f5f6";
}
