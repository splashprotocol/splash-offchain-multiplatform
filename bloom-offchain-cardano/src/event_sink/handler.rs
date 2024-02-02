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
use type_equalities::IsEqual;

use cardano_chain_sync::data::LedgerTxEvent;
use cardano_mempool_sync::data::MempoolUpdate;
use spectrum_cardano_lib::hash::hash_transaction_canonical;
use spectrum_cardano_lib::OutputRef;
use spectrum_offchain::combinators::Ior;
use spectrum_offchain::data::unique_entity::{Confirmed, EitherMod, StateUpdate, Unconfirmed};
use spectrum_offchain::data::{EntitySnapshot, Has, Tradable};
use spectrum_offchain::event_sink::event_handler::EventHandler;
use spectrum_offchain::ledger::TryFromLedger;
use spectrum_offchain::partitioning::Partitioned;

use crate::creds::ExecutorCred;
use crate::event_sink::entity_index::EntityIndex;

#[derive(Copy, Clone, Debug)]
pub struct HandlerContext {
    pub output_ref: OutputRef,
    pub executor_cred: ExecutorCred,
}

impl HandlerContext {
    pub fn new(output_ref: OutputRef, own_cred: ExecutorCred) -> Self {
        Self {
            output_ref,
            executor_cred: own_cred,
        }
    }
}

impl Has<OutputRef> for HandlerContext {
    fn get_labeled<U: IsEqual<OutputRef>>(&self) -> OutputRef {
        self.output_ref
    }
}

impl Has<ExecutorCred> for HandlerContext {
    fn get_labeled<U: IsEqual<ExecutorCred>>(&self) -> ExecutorCred {
        self.executor_cred
    }
}

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
    pub executor_cred: ExecutorCred,
    pub pd: PhantomData<Entity>,
}

impl<const N: usize, PairId, Topic, Entity, Index> PairUpdateHandler<N, PairId, Topic, Entity, Index>
where
    Entity: EntitySnapshot + TryFromLedger<BabbageTransactionOutput, HandlerContext> + Clone,
{
    pub fn new(
        topic: Partitioned<N, PairId, Topic>,
        index: Arc<Mutex<Index>>,
        executor_cred: ExecutorCred,
    ) -> Self {
        Self {
            topic,
            index,
            executor_cred,
            pd: Default::default(),
        }
    }
}

async fn extract_transitions<Entity, Index>(
    index: Arc<Mutex<Index>>,
    executor_cred: ExecutorCred,
    mut tx: BabbageTransaction,
) -> Result<Vec<Ior<Entity, Entity>>, BabbageTransaction>
where
    Entity: EntitySnapshot + TryFromLedger<BabbageTransactionOutput, HandlerContext> + Clone,
    Entity::Version: From<OutputRef>,
    Index: EntityIndex<Entity>,
{
    let num_outputs = tx.body.outputs.len();
    if num_outputs == 0 {
        return Err(tx);
    }
    let mut consumed_entities = HashMap::<Entity::StableId, Entity>::new();
    for i in &tx.body.inputs {
        let state_id = Entity::Version::from(OutputRef::from((i.transaction_id, i.index)));
        let mut index = index.lock().await;
        if index.exists(&state_id) {
            if let Some(entity) = index.get_state(&state_id) {
                let entity_id = entity.stable_id();
                consumed_entities.insert(entity_id, entity);
            }
        }
    }
    let mut produced_entities = HashMap::<Entity::StableId, Entity>::new();
    let tx_hash = hash_transaction_canonical(&tx.body);
    let mut ix = num_outputs - 1;
    let mut non_processed_outputs = vec![];
    while let Some(o) = tx.body.outputs.pop() {
        let o_ref = OutputRef::new(tx_hash, ix as u64);
        match Entity::try_from_ledger(&o, HandlerContext::new(o_ref, executor_cred)) {
            Some(entity) => {
                let entity_id = entity.stable_id();
                produced_entities.insert(entity_id, entity);
            }
            None => {
                non_processed_outputs.push(o);
            }
        }
        ix += 1;
    }
    // Preserve non processed outputs in original ordering.
    tx.body.outputs = non_processed_outputs;

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
        + TryFromLedger<BabbageTransactionOutput, HandlerContext>
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
                match extract_transitions(Arc::clone(&self.index), self.executor_cred, tx).await {
                    Ok(transitions) => {
                        trace!(target: "offchain", "[{}] entities parsed from applied tx", transitions.len());
                        let mut index = self.index.lock().await;
                        if !transitions.is_empty() {
                            index.run_eviction();
                        }
                        for tr in transitions {
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
                match extract_transitions(Arc::clone(&self.index), self.executor_cred, tx).await {
                    Ok(transitions) => {
                        trace!("[{}] entities parsed from unapplied tx", transitions.len());
                        let mut index = self.index.lock().await;
                        if !transitions.is_empty() {
                            index.run_eviction();
                        }
                        for tr in transitions {
                            match &tr {
                                Ior::Left(consumed) => {
                                    index.put_state(consumed.clone());
                                }
                                Ior::Right(produced) => {
                                    index.remove_state(&produced.version());
                                }
                                Ior::Both(consumed, produced) => {
                                    index.put_state(consumed.clone());
                                    index.remove_state(&produced.version());
                                }
                            }
                            let pair = pair_id_of(&tr);
                            let topic = self.topic.get_mut(pair);
                            topic
                                .feed((
                                    pair,
                                    EitherMod::Confirmed(Confirmed(StateUpdate::TransitionRollback(
                                        tr.swap(),
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
impl<const N: usize, PairId, Topic, Entity, Index> EventHandler<MempoolUpdate<BabbageTransaction>>
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
    Index: EntityIndex<Entity>,
{
    async fn try_handle(
        &mut self,
        ev: MempoolUpdate<BabbageTransaction>,
    ) -> Option<MempoolUpdate<BabbageTransaction>> {
        match ev {
            MempoolUpdate::TxAccepted(tx) => {
                match extract_transitions(Arc::clone(&self.index), self.executor_cred, tx).await {
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
                }
            }
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
    use cml_crypto::Ed25519KeyHash;
    use cml_multi_era::babbage::{
        BabbageFormatTxOut, BabbageTransaction, BabbageTransactionBody, BabbageTransactionOutput,
        BabbageTransactionWitnessSet,
    };
    use futures::channel::mpsc;
    use futures::StreamExt;
    use tokio::sync::Mutex;

    use cardano_chain_sync::data::LedgerTxEvent;
    use spectrum_cardano_lib::hash::hash_transaction_canonical;
    use spectrum_cardano_lib::transaction::TransactionOutputExtension;
    use spectrum_cardano_lib::OutputRef;
    use spectrum_offchain::combinators::Ior;
    use spectrum_offchain::data::unique_entity::{Confirmed, EitherMod, StateUpdate};
    use spectrum_offchain::data::{EntitySnapshot, Has, Stable, Tradable};
    use spectrum_offchain::event_sink::event_handler::EventHandler;
    use spectrum_offchain::ledger::TryFromLedger;
    use spectrum_offchain::partitioning::Partitioned;

    use crate::creds::ExecutorCred;
    use crate::event_sink::entity_index::InMemoryEntityIndex;
    use crate::event_sink::handler::PairUpdateHandler;

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
        fn try_from_ledger(repr: &BabbageTransactionOutput, ctx: C) -> Option<Self> {
            Some(TrivialEntity(ctx.get_labeled::<OutputRef>(), repr.value().coin))
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
        let entity_eviction_delay = Duration::from_secs(60 * 5);
        let index = Arc::new(Mutex::new(
            InMemoryEntityIndex::new(entity_eviction_delay).with_tracing(),
        ));
        let (snd, mut recv) = mpsc::channel::<(u8, EitherMod<StateUpdate<TrivialEntity>>)>(100);
        let ex_cred = ExecutorCred(Ed25519KeyHash::from([0u8; 28]));
        let mut handler = PairUpdateHandler::new(Partitioned::new([snd]), index, ex_cred);
        // Handle tx application
        EventHandler::<LedgerTxEvent<BabbageTransaction>>::try_handle(
            &mut handler,
            LedgerTxEvent::TxApplied { tx: tx_1, slot: 0 },
        )
        .await;
        let (_, EitherMod::Confirmed(Confirmed(StateUpdate::Transition(Ior::Right(e1))))) =
            recv.next().await.expect("Must result in new event")
        else {
            panic!("Must be a transition")
        };
        EventHandler::<LedgerTxEvent<BabbageTransaction>>::try_handle(
            &mut handler,
            LedgerTxEvent::TxApplied {
                tx: tx_2.clone(),
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
        EventHandler::<LedgerTxEvent<BabbageTransaction>>::try_handle(
            &mut handler,
            LedgerTxEvent::TxUnapplied(tx_2),
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
