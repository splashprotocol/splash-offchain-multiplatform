use std::collections::{HashMap, HashSet, VecDeque};
use std::fmt::Debug;
use std::hash::Hash;
use std::marker::PhantomData;
use std::sync::Arc;

use async_trait::async_trait;
use cml_crypto::TransactionHash;
use cml_multi_era::babbage::{BabbageTransaction, BabbageTransactionOutput};
use either::Either;
use futures::{Sink, SinkExt};
use log::trace;
use tokio::sync::{Mutex, MutexGuard};

use crate::event_sink::context::{HandlerContext, HandlerContextProto};
use cardano_chain_sync::data::LedgerTxEvent;
use cardano_mempool_sync::data::MempoolUpdate;
use spectrum_cardano_lib::OutputRef;
use spectrum_offchain::combinators::Ior;
use spectrum_offchain::data::order::{OrderUpdate, SpecializedOrder};
use spectrum_offchain::data::unique_entity::{Confirmed, EitherMod, StateUpdate, Unconfirmed};
use spectrum_offchain::data::EntitySnapshot;
use spectrum_offchain::data::Tradable;
use spectrum_offchain::event_sink::event_handler::EventHandler;
use spectrum_offchain::ledger::TryFromLedger;
use spectrum_offchain::partitioning::Partitioned;
use spectrum_offchain_cardano::utxo::ConsumedInputs;

use crate::event_sink::entity_index::TradableEntityIndex;
use crate::event_sink::order_index::OrderIndex;

/// A Tx being processed.
/// Outputs in [BabbageTransaction] may be partially consumed in the process.
pub type ProcessingTransaction = (TransactionHash, BabbageTransaction);

/// A handler for updates that routes resulted [Entity] updates
/// into different topics [Topic] according to partitioning key [PairId].
#[derive(Clone)]
pub struct PairUpdateHandler<const N: usize, PairId, Topic, Entity, Index> {
    pub topic: Partitioned<N, PairId, Topic>,
    /// Index of all non-consumed states of [Entity].
    pub index: Arc<Mutex<Index>>,
    pub context: HandlerContextProto,
    pub pd: PhantomData<Entity>,
}

impl<const N: usize, PairId, Topic, Entity, Index> PairUpdateHandler<N, PairId, Topic, Entity, Index> {
    pub fn new(
        topic: Partitioned<N, PairId, Topic>,
        index: Arc<Mutex<Index>>,
        context: HandlerContextProto,
    ) -> Self {
        Self {
            topic,
            index,
            context,
            pd: Default::default(),
        }
    }
}

#[derive(Clone)]
pub struct SpecializedHandler<H, OrderIndex, Pool> {
    general_handler: H,
    order_index: Arc<Mutex<OrderIndex>>,
    pd: PhantomData<Pool>,
}

impl<H, OrderIndex, Pool> SpecializedHandler<H, OrderIndex, Pool> {
    pub fn new(general_handler: H, order_index: Arc<Mutex<OrderIndex>>) -> Self {
        Self {
            general_handler,
            order_index,
            pd: PhantomData,
        }
    }
}

#[async_trait(?Send)]
impl<const N: usize, PairId, Topic, Pool, Order, PoolIndex, OrderIndex>
    EventHandler<LedgerTxEvent<ProcessingTransaction>>
    for SpecializedHandler<PairUpdateHandler<N, PairId, Topic, Order, PoolIndex>, OrderIndex, Pool>
where
    PairId: Copy + Hash,
    Topic: Sink<(PairId, OrderUpdate<Order, Order>)> + Unpin,
    Topic::Error: Debug,
    Pool: EntitySnapshot + Tradable<PairId = PairId>,
    Order: SpecializedOrder<TPoolId = Pool::StableId>
        + TryFromLedger<BabbageTransactionOutput, HandlerContext>
        + Clone
        + Debug,
    Order::TOrderId: From<OutputRef>,
    OrderIndex: crate::event_sink::order_index::OrderIndex<Order>,
    PoolIndex: TradableEntityIndex<Pool>,
{
    async fn try_handle(
        &mut self,
        ev: LedgerTxEvent<ProcessingTransaction>,
    ) -> Option<LedgerTxEvent<ProcessingTransaction>> {
        match ev {
            LedgerTxEvent::TxApplied { tx, slot } => {
                match extract_atomic_transitions(
                    Arc::clone(&self.order_index),
                    self.general_handler.context,
                    tx,
                )
                .await
                {
                    Ok(transitions) => {
                        trace!(target: "offchain", "[{}] entities parsed from applied tx", transitions.len());
                        let pool_index = self.general_handler.index.lock().await;
                        let mut index = self.order_index.lock().await;
                        index.run_eviction();
                        for tr in transitions {
                            if let Some(pair) = pool_index.pair_of(&pool_ref_of(&tr)) {
                                index_atomic_transition(&mut index, &tr);
                                let topic = self.general_handler.topic.get_mut(pair);
                                topic.feed((pair, tr.into())).await.expect("Channel is closed");
                                topic.flush().await.expect("Failed to commit message");
                            }
                        }
                        None
                    }
                    Err(tx) => Some(LedgerTxEvent::TxApplied { tx, slot }),
                }
            }
            LedgerTxEvent::TxUnapplied(tx) => {
                match extract_atomic_transitions(
                    Arc::clone(&self.order_index),
                    self.general_handler.context,
                    tx,
                )
                .await
                {
                    Ok(transitions) => {
                        trace!("[{}] entities parsed from unapplied tx", transitions.len());
                        let mut index = self.order_index.lock().await;
                        let pool_index = self.general_handler.index.lock().await;
                        index.run_eviction();
                        for tr in transitions {
                            if let Some(pair) = pool_index.pair_of(&pool_ref_of(&tr)) {
                                let inverse_tr = tr.flip();
                                index_atomic_transition(&mut index, &inverse_tr);
                                let topic = self.general_handler.topic.get_mut(pair);
                                topic
                                    .feed((pair, inverse_tr.into()))
                                    .await
                                    .expect("Channel is closed");
                                topic.flush().await.expect("Failed to commit message");
                            }
                        }
                        None
                    }
                    Err(tx) => Some(LedgerTxEvent::TxUnapplied(tx)),
                }
            }
        }
    }
}

#[async_trait(?Send)]
impl<const N: usize, PairId, Topic, Pool, Order, PoolIndex, OrderIndex>
    EventHandler<MempoolUpdate<ProcessingTransaction>>
    for SpecializedHandler<PairUpdateHandler<N, PairId, Topic, Order, PoolIndex>, OrderIndex, Pool>
where
    PairId: Copy + Hash,
    Topic: Sink<(PairId, OrderUpdate<Order, Order>)> + Unpin,
    Topic::Error: Debug,
    Pool: EntitySnapshot + Tradable<PairId = PairId>,
    Order: SpecializedOrder<TPoolId = Pool::StableId>
        + TryFromLedger<BabbageTransactionOutput, HandlerContext>
        + Clone
        + Debug,
    Order::TOrderId: From<OutputRef>,
    OrderIndex: crate::event_sink::order_index::OrderIndex<Order>,
    PoolIndex: TradableEntityIndex<Pool>,
{
    async fn try_handle(
        &mut self,
        ev: MempoolUpdate<ProcessingTransaction>,
    ) -> Option<MempoolUpdate<ProcessingTransaction>> {
        match ev {
            MempoolUpdate::TxAccepted(tx) => {
                match extract_atomic_transitions(
                    Arc::clone(&self.order_index),
                    self.general_handler.context,
                    tx,
                )
                .await
                {
                    Ok(transitions) => {
                        trace!(target: "offchain", "[{}] entities parsed from applied tx", transitions.len());
                        let pool_index = self.general_handler.index.lock().await;
                        let mut index = self.order_index.lock().await;
                        index.run_eviction();
                        for tr in transitions {
                            if let Some(pair) = pool_index.pair_of(&pool_ref_of(&tr)) {
                                index_atomic_transition(&mut index, &tr);
                                let topic = self.general_handler.topic.get_mut(pair);
                                topic.feed((pair, tr.into())).await.expect("Channel is closed");
                                topic.flush().await.expect("Failed to commit message");
                            }
                        }
                        None
                    }
                    Err(tx) => Some(MempoolUpdate::TxAccepted(tx)),
                }
            }
        }
    }
}

fn pool_ref_of<T: SpecializedOrder>(tr: &Either<T, T>) -> T::TPoolId {
    match tr {
        Either::Left(o) => o.get_pool_ref(),
        Either::Right(o) => o.get_pool_ref(),
    }
}

async fn extract_atomic_transitions<Order, Index>(
    index: Arc<Mutex<Index>>,
    context: HandlerContextProto,
    (tx_hash, mut tx): ProcessingTransaction,
) -> Result<Vec<Either<Order, Order>>, ProcessingTransaction>
where
    Order: SpecializedOrder + TryFromLedger<BabbageTransactionOutput, HandlerContext> + Clone,
    Order::TOrderId: From<OutputRef>,
    Index: OrderIndex<Order>,
{
    let num_outputs = tx.body.outputs.len();
    if num_outputs == 0 {
        return Err((tx_hash, tx));
    }
    let mut consumed_entities = HashMap::<Order::TOrderId, Order>::new();
    let mut consumed_utxos = Vec::new();
    for i in &tx.body.inputs {
        let oref = OutputRef::from((i.transaction_id, i.index));
        consumed_utxos.push(oref);
        let state_id = Order::TOrderId::from(oref);
        let mut index = index.lock().await;
        if index.exists(&state_id) {
            if let Some(order) = index.get(&state_id) {
                let entity_id = order.get_self_ref();
                consumed_entities.insert(entity_id, order);
            }
        }
    }
    let mut produced_entities = HashMap::<Order::TOrderId, Order>::new();
    let consumed_utxos = ConsumedInputs::new(consumed_utxos.into_iter());
    let mut ix = num_outputs - 1;
    let mut non_processed_outputs = VecDeque::new();
    while let Some(o) = tx.body.outputs.pop() {
        let o_ref = OutputRef::new(tx_hash, ix as u64);
        match Order::try_from_ledger(&o, &HandlerContext::new(o_ref, consumed_utxos, context)) {
            Some(entity) => {
                trace!(target: "offchain", "extract_atomic_transitions: entity found");
                let entity_id = entity.get_self_ref();
                produced_entities.insert(entity_id, entity);
            }
            None => {
                non_processed_outputs.push_front(o);
            }
        }
        if let Some(next_ix) = ix.checked_sub(1) {
            ix = next_ix;
        }
    }
    // Preserve non-processed outputs in original ordering.
    tx.body.outputs = non_processed_outputs.into();

    // Gather IDs of all recognized entities.
    let mut keys = HashSet::new();
    for k in consumed_entities.keys().chain(produced_entities.keys()) {
        keys.insert(*k);
    }

    // Match consumed versions with produced ones.
    let mut transitions = vec![];
    for k in keys.into_iter() {
        match (consumed_entities.remove(&k), produced_entities.remove(&k)) {
            (Some(consumed), _) => transitions.push(Either::Left(consumed)),
            (_, Some(produced)) => transitions.push(Either::Right(produced)),
            _ => {}
        };
    }

    if transitions.is_empty() {
        return Err((tx_hash, tx));
    }
    Ok(transitions)
}

async fn extract_persistent_transitions<Entity, Index>(
    index: Arc<Mutex<Index>>,
    context: HandlerContextProto,
    (tx_hash, mut tx): ProcessingTransaction,
) -> Result<Vec<Ior<Entity, Entity>>, ProcessingTransaction>
where
    Entity: EntitySnapshot + Tradable + TryFromLedger<BabbageTransactionOutput, HandlerContext> + Clone,
    Entity::Version: From<OutputRef>,
    Index: TradableEntityIndex<Entity>,
{
    let num_outputs = tx.body.outputs.len();
    if num_outputs == 0 {
        return Err((tx_hash, tx));
    }
    let mut consumed_entities = HashMap::<Entity::StableId, Entity>::new();
    let mut consumed_utxos = Vec::new();
    for i in &tx.body.inputs {
        let oref = OutputRef::from((i.transaction_id, i.index));
        consumed_utxos.push(oref);
        let state_id = Entity::Version::from(oref);
        let mut index = index.lock().await;
        if index.exists(&state_id) {
            if let Some(entity) = index.get_state(&state_id) {
                let entity_id = entity.stable_id();
                consumed_entities.insert(entity_id, entity);
            }
        }
    }
    let mut produced_entities = HashMap::<Entity::StableId, Entity>::new();
    trace!(target: "offchain", "+- Scanning Tx {}", tx_hash);
    let mut ix = num_outputs - 1;
    let mut non_processed_outputs = VecDeque::new();
    let consumed_utxos = ConsumedInputs::new(consumed_utxos.into_iter());
    while let Some(o) = tx.body.outputs.pop() {
        let o_ref = OutputRef::new(tx_hash, ix as u64);
        match Entity::try_from_ledger(&o, &HandlerContext::new(o_ref, consumed_utxos, context)) {
            Some(entity) => {
                trace!(target: "offchain", "extract_persistent_transitions: entity found");
                let entity_id = entity.stable_id();
                produced_entities.insert(entity_id, entity);
            }
            None => {
                non_processed_outputs.push_front(o);
            }
        }
        if let Some(next_ix) = ix.checked_sub(1) {
            ix = next_ix;
        }
    }
    // Preserve non-processed outputs in original ordering.
    tx.body.outputs = non_processed_outputs.into();

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
        return Err((tx_hash, tx));
    }
    Ok(transitions)
}

fn pair_id_of<T: Tradable>(xa: &Ior<T, T>) -> T::PairId {
    match xa {
        Ior::Left(o) => o.pair_id(),
        Ior::Right(o) => o.pair_id(),
        Ior::Both(o, _) => o.pair_id(),
    }
}

#[async_trait(?Send)]
impl<const N: usize, PairId, Topic, Entity, Index> EventHandler<LedgerTxEvent<ProcessingTransaction>>
    for PairUpdateHandler<N, PairId, Topic, Entity, Index>
where
    PairId: Copy + Hash,
    Topic: Sink<(PairId, EitherMod<StateUpdate<Entity>>)> + Unpin,
    Topic::Error: Debug,
    Entity: EntitySnapshot
        + Tradable<PairId = PairId>
        + TryFromLedger<BabbageTransactionOutput, HandlerContext>
        + Clone
        + Debug,
    Entity::Version: From<OutputRef>,
    Index: TradableEntityIndex<Entity>,
{
    async fn try_handle(
        &mut self,
        ev: LedgerTxEvent<ProcessingTransaction>,
    ) -> Option<LedgerTxEvent<ProcessingTransaction>> {
        match ev {
            LedgerTxEvent::TxApplied { tx, slot } => {
                match extract_persistent_transitions(Arc::clone(&self.index), self.context, tx).await {
                    Ok(transitions) => {
                        trace!(target: "offchain", "[{}] entities parsed from applied tx", transitions.len());
                        let mut index = self.index.lock().await;
                        index.run_eviction();
                        for tr in transitions {
                            index_transition(&mut index, &tr);
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
            LedgerTxEvent::TxUnapplied(tx) => {
                match extract_persistent_transitions(Arc::clone(&self.index), self.context, tx).await {
                    Ok(transitions) => {
                        trace!("[{}] entities parsed from unapplied tx", transitions.len());
                        let mut index = self.index.lock().await;
                        index.run_eviction();
                        for tr in transitions {
                            let inverse_tr = tr.swap();
                            index_transition(&mut index, &inverse_tr);
                            let pair = pair_id_of(&inverse_tr);
                            let topic = self.topic.get_mut(pair);
                            topic
                                .feed((
                                    pair,
                                    EitherMod::Confirmed(Confirmed(StateUpdate::TransitionRollback(
                                        inverse_tr,
                                    ))),
                                ))
                                .await
                                .expect("Channel is closed");
                            topic.flush().await.expect("Failed to commit message");
                        }
                        None
                    }
                    Err(tx) => Some(LedgerTxEvent::TxUnapplied(tx)),
                }
            }
        }
    }
}

#[async_trait(?Send)]
impl<const N: usize, PairId, Topic, Entity, Index> EventHandler<MempoolUpdate<ProcessingTransaction>>
    for PairUpdateHandler<N, PairId, Topic, Entity, Index>
where
    PairId: Copy + Hash,
    Topic: Sink<(PairId, EitherMod<StateUpdate<Entity>>)> + Unpin,
    Topic::Error: Debug,
    Entity: EntitySnapshot
        + Tradable<PairId = PairId>
        + TryFromLedger<BabbageTransactionOutput, HandlerContext>
        + Clone
        + Debug,
    Entity::Version: From<OutputRef>,
    Index: TradableEntityIndex<Entity>,
{
    async fn try_handle(
        &mut self,
        ev: MempoolUpdate<ProcessingTransaction>,
    ) -> Option<MempoolUpdate<ProcessingTransaction>> {
        match ev {
            MempoolUpdate::TxAccepted(tx) => {
                match extract_persistent_transitions(Arc::clone(&self.index), self.context, tx).await {
                    Ok(transitions) => {
                        trace!("[{}] entities parsed from applied tx", transitions.len());
                        let mut index = self.index.lock().await;
                        index.run_eviction();
                        for tr in transitions {
                            index_transition(&mut index, &tr);
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
                }
            }
        }
    }
}

fn index_atomic_transition<Index, T>(index: &mut MutexGuard<Index>, tr: &Either<T, T>)
where
    T: SpecializedOrder + Clone,
    Index: OrderIndex<T>,
{
    match &tr {
        Either::Left(consumed) => {
            index.register_for_eviction(consumed.get_self_ref());
        }
        Either::Right(produced) => {
            index.put(produced.clone());
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
    use cml_chain::transaction::TransactionInput;
    use cml_crypto::{Ed25519KeyHash, ScriptHash};
    use cml_multi_era::babbage::{
        BabbageFormatTxOut, BabbageTransaction, BabbageTransactionBody, BabbageTransactionOutput,
        BabbageTransactionWitnessSet,
    };
    use futures::channel::mpsc;
    use futures::StreamExt;
    use tokio::sync::Mutex;

    use crate::bounds::Bounds;
    use crate::event_sink::context::HandlerContextProto;
    use algebra_core::monoid::Monoid;
    use cardano_chain_sync::data::LedgerTxEvent;
    use spectrum_cardano_lib::ex_units::ExUnits;
    use spectrum_cardano_lib::hash::hash_transaction_canonical;
    use spectrum_cardano_lib::transaction::TransactionOutputExtension;
    use spectrum_cardano_lib::OutputRef;
    use spectrum_offchain::combinators::Ior;
    use spectrum_offchain::data::unique_entity::{Confirmed, EitherMod, StateUpdate};
    use spectrum_offchain::data::{EntitySnapshot, Has, Stable, Tradable};
    use spectrum_offchain::event_sink::event_handler::EventHandler;
    use spectrum_offchain::ledger::TryFromLedger;
    use spectrum_offchain::partitioning::Partitioned;
    use spectrum_offchain_cardano::creds::OperatorCred;
    use spectrum_offchain_cardano::data::deposit::DepositOrderBounds;
    use spectrum_offchain_cardano::data::redeem::RedeemOrderBounds;
    use spectrum_offchain_cardano::deployment::{DeployedScriptInfo, ProtocolScriptHashes};

    use crate::event_sink::entity_index::InMemoryEntityIndex;
    use crate::event_sink::handler::{PairUpdateHandler, ProcessingTransaction};
    use crate::orders::limit::LimitOrderBounds;

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

    impl<C> TryFromLedger<BabbageTransactionOutput, C> for TrivialEntity
    where
        C: Has<OutputRef>,
    {
        fn try_from_ledger(repr: &BabbageTransactionOutput, ctx: &C) -> Option<Self> {
            Some(TrivialEntity(ctx.select::<OutputRef>(), repr.value().coin))
        }
    }

    #[tokio::test]
    async fn apply_unapply_transaction() {
        let (amt_1, amt_2) = (1000u64, 98000u64);
        let fee = 1000;
        let utxo_1 = BabbageTransactionOutput::BabbageFormatTxOut(BabbageFormatTxOut::new(
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
        let utxo_2 = BabbageTransactionOutput::BabbageFormatTxOut(BabbageFormatTxOut::new(
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
        let tx_1 = BabbageTransaction::new(
            BabbageTransactionBody::new(vec![], vec![utxo_1], fee),
            BabbageTransactionWitnessSet::new(),
            true,
            None,
        );
        let tx_1_hash = hash_transaction_canonical(&tx_1.body);
        let tx_2 = BabbageTransaction::new(
            BabbageTransactionBody::new(vec![TransactionInput::new(tx_1_hash, 0)], vec![utxo_2], 1000),
            BabbageTransactionWitnessSet::new(),
            true,
            None,
        );
        let tx_2_hash = hash_transaction_canonical(&tx_2.body);
        let entity_eviction_delay = Duration::from_secs(60 * 5);
        let index = Arc::new(Mutex::new(
            InMemoryEntityIndex::new(entity_eviction_delay).with_tracing(),
        ));
        let (snd, mut recv) = mpsc::channel::<(u8, EitherMod<StateUpdate<TrivialEntity>>)>(100);
        let ex_cred = OperatorCred(Ed25519KeyHash::from([0u8; 28]));
        let context = HandlerContextProto {
            bounds: Bounds {
                limit_order: LimitOrderBounds {
                    min_cost_per_ex_step: 1000,
                },
                deposit_order: DepositOrderBounds {
                    min_collateral_ada: 1000,
                },
                redeem_order: RedeemOrderBounds {
                    min_collateral_ada: 1000,
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
            },
        };
        let mut handler = PairUpdateHandler::new(Partitioned::new([snd]), index, context);
        // Handle tx application
        EventHandler::<LedgerTxEvent<ProcessingTransaction>>::try_handle(
            &mut handler,
            LedgerTxEvent::TxApplied {
                tx: (tx_1_hash, tx_1),
                slot: 0,
            },
        )
        .await;
        let (_, EitherMod::Confirmed(Confirmed(StateUpdate::Transition(Ior::Right(e1))))) =
            recv.next().await.expect("Must result in new event")
        else {
            panic!("Must be a transition")
        };
        EventHandler::<LedgerTxEvent<ProcessingTransaction>>::try_handle(
            &mut handler,
            LedgerTxEvent::TxApplied {
                tx: (tx_2_hash, tx_2.clone()),
                slot: 1,
            },
        )
        .await;
        let (_, EitherMod::Confirmed(Confirmed(StateUpdate::Transition(Ior::Both(e1_reversed, e2))))) =
            recv.next().await.expect("Must result in new event")
        else {
            panic!("Must be a transition")
        };
        assert_eq!(e1_reversed, e1);
        EventHandler::<LedgerTxEvent<ProcessingTransaction>>::try_handle(
            &mut handler,
            LedgerTxEvent::TxUnapplied((tx_2_hash, tx_2)),
        )
        .await;
        let (
            _,
            EitherMod::Confirmed(Confirmed(StateUpdate::TransitionRollback(Ior::Both(
                e2_reversed,
                e1_revived,
            )))),
        ) = recv.next().await.expect("Must result in new event")
        else {
            panic!("Must be a transition")
        };
        assert_eq!(e2_reversed, e2);
        assert_eq!(e1_revived, e1);
    }
}
