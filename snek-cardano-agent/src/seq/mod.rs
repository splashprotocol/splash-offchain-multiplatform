mod cond;
mod session;

use crate::seq::cond::{ConditionalValidation, Id, Validations};
use crate::seq::session::{SessionInProgress, SessionRejection};
use bloom_offchain_cardano::event_sink::handler::LedgerCx;
use cml_core::Slot;
use futures::Stream;
use log::{info, trace};
use spectrum_offchain::data::ior::Ior;
use spectrum_offchain::domain::event::{Channel, Confirmed, Transition};
use spectrum_offchain::domain::{SeqState, Stable};
use spectrum_offchain_cardano::data::pair::PairId;
use spectrum_offchain_cardano::raw_bytes::RawBytes;
use std::collections::hash_map::Entry;
use std::collections::{btree_map, BTreeMap, HashMap, HashSet, VecDeque};
use std::fmt::Display;
use std::hash::Hash;
use std::pin::Pin;
use std::task::{Context, Poll};

/// Use ledger to sequence events deterministically.
pub fn with_sequencing<Ticks, Events, K, T>(
    clock_ticks: Ticks,
    events: Events,
    session_duration: Slot,
    session_settlement: Slot,
    disable: bool,
) -> WithDeterministicSeq<Ticks, Events, K, T>
where
    Ticks: Stream<Item = Slot> + Unpin,
    Events: Stream<Item = (PairId, Channel<Transition<T>, LedgerCx>)> + Unpin,
    K: Copy + Eq + Hash + Ord + Unpin,
    T: SeqState<StableId = K> + Unpin,
{
    WithDeterministicSeq::new(clock_ticks, events, session_duration, session_settlement, disable)
}

pub struct WithDeterministicSeq<Ticks, Events, K, T> {
    clock_ticks: Ticks,
    events: Events,
    current_slot: Slot,
    /// Events that didn't get into an active session, will be released after the session duration
    delayed_events: BTreeMap<Slot, Vec<(PairId, Channel<Transition<T>, LedgerCx>)>>,
    /// Just released events
    pending_events: VecDeque<(PairId, Channel<Transition<T>, LedgerCx>)>,
    /// Event that is waiting to be processed after pending events
    stashed_event: Option<(PairId, Channel<Transition<T>, LedgerCx>)>,
    completed_sessions: HashSet<PairId>,
    active_sessions: HashMap<PairId, SessionInProgress<K, T>>,
    session_duration: Slot,
    session_settlement: Slot,
    disable: bool,
}

impl<Ticks, Events, K, T> WithDeterministicSeq<Ticks, Events, K, T> {
    pub fn new(
        clock_ticks: Ticks,
        events: Events,
        session_duration: Slot,
        session_settlement: Slot,
        disable: bool,
    ) -> Self {
        Self {
            clock_ticks,
            events,
            current_slot: 0,
            delayed_events: BTreeMap::new(),
            pending_events: VecDeque::new(),
            stashed_event: None,
            completed_sessions: HashSet::new(),
            active_sessions: HashMap::new(),
            session_duration,
            session_settlement,
            disable,
        }
    }

    fn update_clocks(&mut self, slot: Slot) -> bool
    where
        K: Copy + Eq + Ord + Hash + Display + RawBytes,
        T: Stable<StableId = K>,
    {
        let upgrade = match slot.cmp(&self.current_slot) {
            std::cmp::Ordering::Greater => {
                self.current_slot = slot;
                Some(slot)
            }
            _ => None,
        };
        match upgrade {
            None => false,
            Some(upgraded_to) => {
                let mut closed_sessions = vec![];
                let mut pending_events = vec![];

                // Process active sessions
                for (pair, sess) in self.active_sessions.iter_mut() {
                    if let Some(released_events) = sess.upgrade(upgraded_to) {
                        info!("Pair {} graduated at {}", pair, upgraded_to);
                        closed_sessions.push(*pair);
                        pending_events.push((*pair, released_events));
                    }
                }

                // Process delayed events that have reached their release time
                let delayed_slots: Vec<_> = self
                    .delayed_events
                    .range(..=upgraded_to)
                    .map(|(slot, _)| *slot)
                    .collect();

                for slot in delayed_slots {
                    if let Some(events) = self.delayed_events.remove(&slot) {
                        trace!("Releasing {} delayed events from slot {}", events.len(), slot);
                        for (pair, event) in events {
                            self.pending_events.push_back((pair, event));
                        }
                    }
                }
                for key in closed_sessions {
                    self.active_sessions.remove(&key);
                    self.completed_sessions.insert(key);
                }
                for (pair, events) in pending_events {
                    for event in events {
                        self.pending_events.push_back((pair, event));
                    }
                }
                true
            }
        }
    }

    fn update_session(&mut self, pair: PairId, event: Channel<Transition<T>, LedgerCx>)
    where
        K: Copy + Eq + Hash + Display + Unpin,
        T: SeqState<StableId = K>
            + ConditionalValidation<{ Validations::HypedLaunch as u8 }, ()>
            + ConditionalValidation<{ Validations::CancellationLock as u8 }, Slot>
            + Unpin,
    {
        match self.active_sessions.entry(pair) {
            Entry::Vacant(entry) => {
                match event {
                    Channel::Ledger(Confirmed(Transition::Forward(Ior::Right(state))), cx)
                        if state.is_quasi_permanent() && state.is_initial() =>
                    {
                        // New session is triggered
                        let session_sealed_at = self.current_slot + self.session_duration;
                        let capped = state.cond(Id::<{ Validations::HypedLaunch as u8 }>);
                        trace!(
                            "New session {} created at {}, sealed at {}, capped = {}",
                            pair,
                            self.current_slot,
                            session_sealed_at,
                            capped
                        );
                        entry.insert(SessionInProgress::new(
                            Transition::Forward(Ior::Right(state)),
                            cx,
                            session_sealed_at,
                            self.session_settlement,
                            capped,
                        ));
                    }
                    // Session wasn't triggered, add event to delayed_events
                    _ => self.delay_event(pair, event),
                }
            }
            Entry::Occupied(mut entry) => {
                match entry.get_mut().register_event(event) {
                    Ok(_) => (),
                    Err(SessionRejection::SessionCancelled) => {
                        trace!("Session trigger was rolled back, discarded session {}", pair);
                        // Session trigger was rolled back, discard session.
                        entry.remove();
                    }
                    Err(SessionRejection::EventRejected(event)) => {
                        self.delay_event(pair, event);
                    }
                }
            }
        }
    }

    fn delay_event(&mut self, pair: PairId, event: Channel<Transition<T>, LedgerCx>)
    where
        K: Display,
        T: Stable<StableId = K>,
    {
        let release_slot = self.current_slot + self.session_duration;
        trace!(
            "Event {} for pair {} delayed until slot {}",
            event.stable_id(),
            pair,
            release_slot
        );
        match self.delayed_events.entry(release_slot) {
            btree_map::Entry::Vacant(entry) => {
                entry.insert(vec![(pair, event)]);
            }
            btree_map::Entry::Occupied(mut entry) => {
                entry.get_mut().push((pair, event));
            }
        }
    }
}

impl<Ticks, Events, K, T> Stream for WithDeterministicSeq<Ticks, Events, K, T>
where
    Ticks: Stream<Item = Slot> + Unpin,
    Events: Stream<Item = (PairId, Channel<Transition<T>, LedgerCx>)> + Unpin,
    K: Copy + Eq + Hash + Ord + Display + Unpin + RawBytes,
    T: SeqState<StableId = K>
        + ConditionalValidation<{ Validations::HypedLaunch as u8 }, ()>
        + ConditionalValidation<{ Validations::CancellationLock as u8 }, Slot>
        + Unpin,
{
    type Item = (PairId, Channel<Transition<T>, LedgerCx>);

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context) -> Poll<Option<Self::Item>> {
        loop {
            // Yield seasoned events first.
            if let Some(event) = self.pending_events.pop_front() {
                return Poll::Ready(Some(event));
            }
            if let Poll::Ready(Some(new_slot)) = Stream::poll_next(Pin::new(&mut self.clock_ticks), cx) {
                if self.update_clocks(new_slot) {
                    continue;
                }
            }
            if let Some((pair, event)) = self.stashed_event.take() {
                if self.completed_sessions.contains(&pair) {
                    return Poll::Ready(Some((pair, event)));
                }
                self.update_session(pair, event);
            }
            if let Poll::Ready(Some((pair, event))) = Stream::poll_next(Pin::new(&mut self.events), cx) {
                if let Channel::Ledger(_, lcx) = &event {
                    if self.update_clocks(lcx.slot) {
                        // Events were released in result of an update, stash current event and yield them first.
                        self.stashed_event.replace((pair, event));
                        continue;
                    }
                }
                if self.completed_sessions.contains(&pair) || self.disable {
                    trace!("Passing event {}", event.stable_id());
                    return Poll::Ready(Some((pair, event)));
                }
                self.update_session(pair, event);
                continue;
            }
            break;
        }
        Poll::Pending
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cml_crypto::BlockHeaderHash;
    use futures::channel::mpsc;
    use futures::{SinkExt, StreamExt};
    use spectrum_cardano_lib::{AssetClass, Token};
    use spectrum_offchain::data::ior::Ior;
    use spectrum_offchain::domain::event::{Channel, Confirmed, Transition, Unconfirmed};
    use spectrum_offchain::domain::Stable;
    use std::collections::HashSet;

    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    enum TestEvent {
        Order { id: u64, init: bool },
        Pool { id: u64, init: bool },
    }

    impl Stable for TestEvent {
        type StableId = u64;

        fn stable_id(&self) -> Self::StableId {
            match self {
                TestEvent::Order { id, .. } => *id,
                TestEvent::Pool { id, .. } => *id,
            }
        }

        fn is_quasi_permanent(&self) -> bool {
            match self {
                TestEvent::Order { .. } => false,
                TestEvent::Pool { .. } => true,
            }
        }
    }

    impl SeqState for TestEvent {
        fn is_initial(&self) -> bool {
            match self {
                TestEvent::Order { init, .. } => *init,
                TestEvent::Pool { init, .. } => *init,
            }
        }
    }

    impl ConditionalValidation<{ Validations::HypedLaunch as u8 }, ()> for TestEvent {
        fn cond(&self, id: Id<{ Validations::HypedLaunch as u8 }>) -> bool {
            match self {
                TestEvent::Order { .. } => false,
                TestEvent::Pool { id, .. } => *id == 99,
            }
        }

        fn is_valid(&self, id: Id<{ Validations::HypedLaunch as u8 }>, cx: ()) -> bool {
            match self {
                TestEvent::Order { id, .. } => *id == 98,
                TestEvent::Pool { .. } => true,
            }
        }
    }

    impl ConditionalValidation<{ Validations::CancellationLock as u8 }, Slot> for TestEvent {
        fn cond(&self, id: Id<{ Validations::CancellationLock as u8 }>) -> bool {
            match self {
                TestEvent::Order { .. } => false,
                TestEvent::Pool { id, .. } => true,
            }
        }

        fn is_valid(&self, id: Id<{ Validations::CancellationLock as u8 }>, cx: Slot) -> bool {
            match self {
                TestEvent::Order { id, .. } => true,
                TestEvent::Pool { .. } => true,
            }
        }
    }

    #[tokio::test]
    async fn session_projection_finalize_by_event() {
        let (_, tick_rx) = mpsc::channel(50);
        let (mut event_tx, event_rx) = mpsc::channel(50);

        let session_duration = 10;
        let session_settlement = 5;

        let pair_id_1 = PairId::canonical(
            AssetClass::Native,
            AssetClass::Token(Token::from_string_unsafe(
                "12536cb877860b2ed0d532f24ba170d084fe958ea02087f4540f2cd7.",
            )),
        );
        let pair_id_2 = PairId::canonical(
            AssetClass::Native,
            AssetClass::Token(Token::from_string_unsafe(
                "74e504f2e09170f898c61a0f81463d5da60fe86bf9dafede80f24f35.",
            )),
        );

        let timeout = std::time::Duration::from_millis(100);
        let mut stream =
            WithDeterministicSeq::new(tick_rx, event_rx, session_duration, session_settlement, false);

        // Simulated events
        // Event 0: Starting a new session
        let state_0 = TestEvent::Pool { id: 1, init: true };
        let event_0 = Channel::Mempool(Unconfirmed(Transition::Forward(Ior::Right(state_0))));
        event_tx.send((pair_id_1, event_0)).await.unwrap();

        let _ = tokio::time::timeout(timeout, stream.next()).await;

        // Unconfirmed pool doesn't trigger a session start
        assert!(stream.active_sessions.is_empty());

        // Event 1: Starting a new session
        let state_1 = TestEvent::Pool { id: 1, init: true };
        let event_1 = Channel::Ledger(
            Confirmed(Transition::Forward(Ior::Right(state_1))),
            LedgerCx {
                slot: 5,
                block_hash: BlockHeaderHash::from([0u8; 32]),
            },
        );
        event_tx.send((pair_id_1, event_1)).await.unwrap();

        let _ = tokio::time::timeout(timeout, stream.next()).await;

        // Now a new session is triggered
        assert!(stream.active_sessions.contains_key(&pair_id_1));

        // Event 2: For a different session
        let state_2 = TestEvent::Pool { id: 2, init: true };
        let event_2 = Channel::Ledger(
            Confirmed(Transition::Forward(Ior::Right(state_2))),
            LedgerCx {
                slot: 12,
                block_hash: BlockHeaderHash::from([1u8; 32]),
            },
        );
        event_tx.send((pair_id_2, event_2)).await.unwrap();

        // Event 3: Update event for an existing session
        let state_3 = TestEvent::Order { id: 3, init: false };
        let event_3 = Channel::Ledger(
            Confirmed(Transition::Forward(Ior::Right(state_3))),
            LedgerCx {
                slot: 15,
                block_hash: BlockHeaderHash::from([2u8; 32]),
            },
        );
        event_tx.send((pair_id_1, event_3)).await.unwrap();

        // Event 4: Another order event for the same session
        let state_4 = TestEvent::Order { id: 4, init: false };
        let event_4 = Channel::Ledger(
            Confirmed(Transition::Forward(Ior::Right(state_4))),
            LedgerCx {
                slot: 18,
                block_hash: BlockHeaderHash::from([3u8; 32]),
            },
        );
        event_tx.send((pair_id_1, event_4)).await.unwrap();

        // Event 5: Final order event leading to session completion
        // This order itself won't get into session window.
        let state_5 = TestEvent::Order { id: 5, init: false };
        let event_5 = Channel::Ledger(
            Confirmed(Transition::Forward(Ior::Right(state_5))),
            LedgerCx {
                slot: 20,
                block_hash: BlockHeaderHash::from([4u8; 32]),
            },
        );
        event_tx.send((pair_id_1, event_5)).await.unwrap();

        let mut yielded_events = vec![];
        while let Some((pair_id, event)) = tokio::time::timeout(timeout, stream.next()).await.unwrap_or(None)
        {
            yielded_events.push((pair_id, event));
        }

        assert_eq!(yielded_events.len(), 4);
        let pair_ids: HashSet<_> = yielded_events.iter().map(|(pair_id, _)| pair_id).collect();
        assert!(pair_ids.contains(&pair_id_1));
        assert!(!pair_ids.contains(&pair_id_2));
    }

    #[tokio::test]
    async fn session_projection_finalize_by_tick() {
        let (mut tick_tx, tick_rx) = mpsc::channel(50);
        let (mut event_tx, event_rx) = mpsc::channel(50);

        let session_duration = 10; // Duration of the session
        let session_settlement = 5; // Grace period for settlement after session completes

        let pair_id_1 = PairId::canonical(
            AssetClass::Native,
            AssetClass::Token(Token::from_string_unsafe(
                "12536cb877860b2ed0d532f24ba170d084fe958ea02087f4540f2cd7.",
            )),
        );

        let timeout = std::time::Duration::from_millis(100);
        let mut stream =
            WithDeterministicSeq::new(tick_rx, event_rx, session_duration, session_settlement, false);

        // Event 1: Starting a new session
        let state_1 = TestEvent::Pool { id: 1, init: true };
        let event_1 = Channel::Ledger(
            Confirmed(Transition::Forward(Ior::Right(state_1))),
            LedgerCx {
                slot: 5,
                block_hash: BlockHeaderHash::from([0u8; 32]),
            },
        );
        event_tx.send((pair_id_1, event_1)).await.unwrap();

        // Event 3: Update event for an existing session
        let state_3 = TestEvent::Order { id: 3, init: false };
        let event_3 = Channel::Ledger(
            Confirmed(Transition::Forward(Ior::Right(state_3))),
            LedgerCx {
                slot: 15,
                block_hash: BlockHeaderHash::from([2u8; 32]),
            },
        );
        event_tx.send((pair_id_1, event_3)).await.unwrap();

        // Wait for the session to be triggered
        let _ = tokio::time::timeout(timeout, stream.next()).await;
        assert!(stream.active_sessions.contains_key(&pair_id_1));

        // Simulating ticks to progress time
        tick_tx.send(19).await.unwrap();

        // Session should not be finalized yet
        let _ = tokio::time::timeout(timeout, stream.next()).await;
        assert!(stream.active_sessions.contains_key(&pair_id_1));

        // Now session should finalize
        tick_tx.send(20).await.unwrap();

        // After enough ticks, the session should complete due to timeout
        let mut yielded_events = vec![];
        while let Some((pair_id, event)) = tokio::time::timeout(timeout, stream.next()).await.unwrap_or(None)
        {
            yielded_events.push((pair_id, event));
        }

        // Ensure the session finalized successfully for `pair_id_1`
        assert_eq!(yielded_events.len(), 2);
        let pair_ids: HashSet<_> = yielded_events.iter().map(|(pair_id, _)| pair_id).collect();
        assert!(pair_ids.contains(&pair_id_1));
    }

    #[tokio::test]
    async fn multiple_sessions_finalize_by_tick() {
        let (mut tick_tx, tick_rx) = mpsc::channel(50);
        let (mut event_tx, event_rx) = mpsc::channel(50);

        let session_duration = 10; // Duration of the session
        let session_settlement = 5; // Grace period for settlement after session completes

        let pair_id_1 = PairId::canonical(
            AssetClass::Native,
            AssetClass::Token(Token::from_string_unsafe(
                "12536cb877860b2ed0d532f24ba170d084fe958ea02087f4540f2cd7.",
            )),
        );

        let pair_id_2 = PairId::canonical(
            AssetClass::Native,
            AssetClass::Token(Token::from_string_unsafe(
                "22536cb877860b2ed0d532f24ba170d084fe958ea02087f4540f2cd7.",
            )),
        );

        let timeout = std::time::Duration::from_millis(100);
        let mut stream =
            WithDeterministicSeq::new(tick_rx, event_rx, session_duration, session_settlement, false);

        // Event 1: Starting session for pair_id_1
        let state_1 = TestEvent::Pool { id: 1, init: true };
        let event_1 = Channel::Ledger(
            Confirmed(Transition::Forward(Ior::Right(state_1))),
            LedgerCx {
                slot: 5,
                block_hash: BlockHeaderHash::from([0u8; 32]),
            },
        );
        event_tx.send((pair_id_1, event_1)).await.unwrap();

        // Event 2: Starting session for pair_id_2
        let state_2 = TestEvent::Pool { id: 2, init: true };
        let event_2 = Channel::Ledger(
            Confirmed(Transition::Forward(Ior::Right(state_2))),
            LedgerCx {
                slot: 5,
                block_hash: BlockHeaderHash::from([1u8; 32]),
            },
        );
        event_tx.send((pair_id_2, event_2)).await.unwrap();

        // Event 3: Update event for pair_id_1
        let state_3 = TestEvent::Order { id: 3, init: true };
        let event_3 = Channel::Ledger(
            Confirmed(Transition::Forward(Ior::Right(state_3))),
            LedgerCx {
                slot: 15,
                block_hash: BlockHeaderHash::from([2u8; 32]),
            },
        );
        event_tx.send((pair_id_1, event_3)).await.unwrap();

        // Event 4: Update event for pair_id_1
        let state_4 = TestEvent::Order { id: 4, init: true };
        let event_4 = Channel::Ledger(
            Confirmed(Transition::Forward(Ior::Right(state_4))),
            LedgerCx {
                slot: 15,
                block_hash: BlockHeaderHash::from([4u8; 32]),
            },
        );
        event_tx.send((pair_id_2, event_4)).await.unwrap();

        // Wait for sessions to be triggered
        let _ = tokio::time::timeout(timeout, stream.next()).await;
        let _ = tokio::time::timeout(timeout, stream.next()).await;

        assert!(stream.active_sessions.contains_key(&pair_id_1));
        assert!(stream.active_sessions.contains_key(&pair_id_2));

        // Simulate ticks to progress time beyond session duration for both sessions.
        tick_tx.send(19).await.unwrap();

        // Sessions should not be finalized yet
        let _ = tokio::time::timeout(timeout, stream.next()).await;
        assert!(stream.active_sessions.contains_key(&pair_id_1));
        assert!(stream.active_sessions.contains_key(&pair_id_2));

        // Finalizing both sessions by advancing the ticks to their respective timeouts.
        tick_tx.send(20).await.unwrap();

        // After enough ticks, both sessions should finalize
        let mut yielded_events = vec![];
        while let Some((pair_id, event)) = tokio::time::timeout(timeout, stream.next()).await.unwrap_or(None)
        {
            yielded_events.push((pair_id, event));
        }

        // Ensure both sessions finalized successfully
        assert_eq!(yielded_events.len(), 4); // 2 events each for finalization
        let pair_ids: HashSet<_> = yielded_events.iter().map(|(pair_id, _)| pair_id).collect();
        assert!(pair_ids.contains(&pair_id_1));
        assert!(pair_ids.contains(&pair_id_2));
    }

    #[tokio::test]
    async fn session_does_not_contain_invalid_events() {
        let (_, tick_rx) = mpsc::channel(50);
        let (mut event_tx, event_rx) = mpsc::channel(50);

        let session_duration = 10;
        let session_settlement = 5;

        let pair_id_1 = PairId::canonical(
            AssetClass::Native,
            AssetClass::Token(Token::from_string_unsafe(
                "12536cb877860b2ed0d532f24ba170d084fe958ea02087f4540f2cd7.",
            )),
        );

        let timeout = std::time::Duration::from_millis(100);
        let mut stream =
            WithDeterministicSeq::new(tick_rx, event_rx, session_duration, session_settlement, false);

        // Simulated events
        // Event 0: Starting a new session
        let state_0 = TestEvent::Pool { id: 99, init: true };
        let event_0 = Channel::Mempool(Unconfirmed(Transition::Forward(Ior::Right(state_0))));
        event_tx.send((pair_id_1, event_0)).await.unwrap();

        let _ = tokio::time::timeout(timeout, stream.next()).await;

        // Unconfirmed pool doesn't trigger a session start
        assert!(stream.active_sessions.is_empty());

        // Event 1: Starting a new session
        let state_1 = TestEvent::Pool { id: 99, init: true };
        let event_1 = Channel::Ledger(
            Confirmed(Transition::Forward(Ior::Right(state_1))),
            LedgerCx {
                slot: 5,
                block_hash: BlockHeaderHash::from([0u8; 32]),
            },
        );
        event_tx.send((pair_id_1, event_1)).await.unwrap();

        let _ = tokio::time::timeout(timeout, stream.next()).await;

        // Now a new session is triggered
        assert!(stream.active_sessions.contains_key(&pair_id_1));

        let state_3 = TestEvent::Order { id: 98, init: true };
        let event_3 = Channel::Ledger(
            Confirmed(Transition::Forward(Ior::Right(state_3))),
            LedgerCx {
                slot: 15,
                block_hash: BlockHeaderHash::from([2u8; 32]),
            },
        );
        event_tx.send((pair_id_1, event_3)).await.unwrap();

        // Event 4: Another order event for the same session
        let state_4 = TestEvent::Order { id: 4, init: false };
        let event_4 = Channel::Ledger(
            Confirmed(Transition::Forward(Ior::Right(state_4))),
            LedgerCx {
                slot: 18,
                block_hash: BlockHeaderHash::from([3u8; 32]),
            },
        );
        event_tx.send((pair_id_1, event_4)).await.unwrap();

        // Event 5: Final order event leading to session completion
        // This order itself won't get into session window.
        let state_5 = TestEvent::Order { id: 5, init: false };
        let event_5 = Channel::Ledger(
            Confirmed(Transition::Forward(Ior::Right(state_5))),
            LedgerCx {
                slot: 20,
                block_hash: BlockHeaderHash::from([4u8; 32]),
            },
        );
        event_tx.send((pair_id_1, event_5)).await.unwrap();

        let mut yielded_events = vec![];
        while let Some((pair_id, event)) = tokio::time::timeout(timeout, stream.next()).await.unwrap_or(None)
        {
            yielded_events.push((pair_id, event));
        }

        assert_eq!(yielded_events.len(), 3);
        let pair_ids: HashSet<_> = yielded_events.iter().map(|(pair_id, _)| pair_id).collect();
        assert!(pair_ids.contains(&pair_id_1));
    }

    #[tokio::test]
    async fn delayed_non_triggering_event_is_released_after_delay() {
        let (mut tick_tx, tick_rx) = mpsc::channel(50);
        let (mut event_tx, event_rx) = mpsc::channel(50);

        let session_duration = 10;
        let session_settlement = 5;

        let pair_id = PairId::canonical(
            AssetClass::Native,
            AssetClass::Token(Token::from_string_unsafe(
                "deadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef.",
            )),
        );

        let timeout = std::time::Duration::from_millis(100);
        let mut stream =
            WithDeterministicSeq::new(tick_rx, event_rx, session_duration, session_settlement, false);

        let ids = [42, 43];

        // Send a non-triggering ledger event (Order) at slot 5
        let state = TestEvent::Order {
            id: ids[0],
            init: false,
        };
        let event = Channel::Ledger(
            Confirmed(Transition::Forward(Ior::Right(state))),
            LedgerCx {
                slot: 5,
                block_hash: BlockHeaderHash::from([9u8; 32]),
            },
        );
        event_tx.send((pair_id, event)).await.unwrap();

        // Drive the stream once; it should not yield the delayed event yet
        let _ = tokio::time::timeout(timeout, stream.next()).await;

        // Send a non-triggering ledger event (Order) at slot 5
        let state = TestEvent::Order {
            id: ids[1],
            init: false,
        };
        let event = Channel::Mempool(Unconfirmed(Transition::Forward(Ior::Right(state))));
        event_tx.send((pair_id, event)).await.unwrap();

        // Drive the stream once; it should not yield the delayed event yet
        let _ = tokio::time::timeout(timeout, stream.next()).await;

        // No session should be created
        assert!(stream.active_sessions.is_empty());

        // Advance time to just before the release slot (5 + 10 = 15)
        tick_tx.send(14).await.unwrap();
        let _ = tokio::time::timeout(timeout, stream.next()).await;

        // Still nothing should be yielded
        let _ = tokio::time::timeout(timeout, stream.next()).await;

        // Now advance to the release slot
        tick_tx.send(15).await.unwrap();

        // Collect yielded events
        let mut yielded = vec![];
        while let Some((pid, ev)) = tokio::time::timeout(timeout, stream.next()).await.unwrap_or(None) {
            yielded.push((pid, ev));
        }

        assert_eq!(yielded.len(), 2);
        let pair_ids: HashSet<_> = yielded.iter().map(|(_, ev)| ev.stable_id()).collect();
        assert!(ids.iter().all(|id| pair_ids.contains(id)));
        // Ensure still no sessions
        assert!(stream.active_sessions.is_empty());
    }
}
