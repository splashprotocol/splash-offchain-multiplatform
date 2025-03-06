use bloom_offchain_cardano::event_sink::handler::LedgerCx;
use cml_core::Slot;
use cml_crypto::BlockHeaderHash;
use spectrum_offchain::data::ior::Ior;
use spectrum_offchain::domain::event::{Channel, Confirmed, Transition};
use spectrum_offchain::domain::{SeqState, Stable};
use spectrum_offchain::partitioning::hash_partitioning_key;
use std::collections::hash_map::Entry;
use std::collections::{HashMap, VecDeque};
use std::hash::Hash;

pub(crate) struct SessionInProgress<K, T> {
    opening_event: K,
    opening_event_cx: LedgerCx,
    original_ordering: VecDeque<K>,
    confirmation_ordering: VecDeque<(K, Slot)>,
    event_registry: HashMap<K, Channel<Transition<T>, LedgerCx>>,
    sealed_at: Slot,
    settlement_delay: Slot,
}

impl<K, T> SessionInProgress<K, T> {
    pub(crate) fn new(event: Transition<T>, cx: LedgerCx, sealed_at: Slot, settlement_delay: Slot) -> Self
    where
        K: Copy + Eq + Hash,
        T: SeqState<StableId = K>,
    {
        let key = event.stable_id();
        SessionInProgress {
            opening_event: key,
            opening_event_cx: cx,
            original_ordering: VecDeque::new(),
            confirmation_ordering: VecDeque::new(),
            event_registry: HashMap::from([(key, Channel::ledger(event, cx))]),
            sealed_at,
            settlement_delay,
        }
    }

    pub(crate) fn register_event(&mut self, event: Channel<Transition<T>, LedgerCx>) -> Result<(), ()>
    where
        K: Copy + Eq + Hash,
        T: Stable<StableId = K>,
    {
        let event_key = event.stable_id();
        match self.event_registry.entry(event_key) {
            Entry::Occupied(mut entry) => {
                let current = entry.get();
                if is_cancellation(&event) {
                    self.original_ordering.retain(|k| *k != event_key);
                    entry.remove();
                    if event_key == self.opening_event {
                        return Err(());
                    }
                } else {
                    if let Some(confirmed_at) = is_confirmation(current, &event) {
                        self.confirmation_ordering.push_back((event_key, confirmed_at));
                        entry.insert(event);
                    }
                }
            }
            Entry::Vacant(entry) => {
                if !is_cancellation(&event) {
                    if let Channel::Ledger(Confirmed(Transition::Forward(_)), lcx) = &event {
                        self.confirmation_ordering.push_back((event_key, lcx.slot));
                    } else {
                        self.original_ordering.push_back(event_key);
                    }
                    entry.insert(event);
                }
            }
        }
        Ok(())
    }

    /// Clock upgrade may result in finalization of the session
    /// and accumulated events being released.
    pub(crate) fn upgrade(&mut self, slot: Slot) -> Option<Vec<Channel<Transition<T>, LedgerCx>>>
    where
        K: Copy + Eq + Ord + Hash,
        T: Stable<StableId = K>,
    {
        if slot >= self.sealed_at + self.settlement_delay {
            let mut settled_events = vec![];
            let mut remaining_events = vec![];
            if let Some(event) = self.event_registry.remove(&self.opening_event) {
                settled_events.push(event);
            }
            while let Some((key, s)) = self.confirmation_ordering.pop_front() {
                if let Some(event) = self.event_registry.remove(&key) {
                    if s <= self.sealed_at {
                        settled_events.push(event);
                    } else {
                        remaining_events.push(event);
                    }
                }
            }
            while let Some(key) = self.original_ordering.pop_front() {
                if let Some(event) = self.event_registry.remove(&key) {
                    remaining_events.push(event);
                }
            }

            // Apply deterministic sequencing
            let num_settled_events = settled_events.len();
            let window_size = seq_window_size(num_settled_events, self.opening_event_cx.block_hash);
            settled_events[..window_size].sort_by(|a, b| a.stable_id().cmp(&b.stable_id()));

            return Some(
                settled_events
                    .into_iter()
                    .chain(remaining_events.into_iter())
                    .collect(),
            );
        }
        None
    }
}

// Determine sequencing window based on deterministic block data
fn seq_window_size(num_settled_events: usize, block_hash: BlockHeaderHash) -> usize {
    (hash_partitioning_key(block_hash) % (num_settled_events as u64)) as usize
}

fn is_confirmation<T>(
    base: &Channel<Transition<T>, LedgerCx>,
    new: &Channel<Transition<T>, LedgerCx>,
) -> Option<Slot> {
    if !matches!(base, Channel::Ledger(_, _)) {
        if let Channel::Ledger(Confirmed(Transition::Forward(_)), lcx) = new {
            return Some(lcx.slot);
        }
    }
    None
}

fn is_cancellation<T, C>(new: &Channel<Transition<T>, C>) -> bool {
    match new.erased() {
        Transition::Forward(Ior::Left(_)) | Transition::Backward(Ior::Left(_)) => true,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use cml_crypto::BlockHeaderHash;
    use rand::RngCore;
    use spectrum_offchain::data::ior::Ior;
    use spectrum_offchain::domain::{SeqState, Stable};

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

    #[test]
    fn test_session_in_progress_event_cancel_out() {
        use super::SessionInProgress;
        use bloom_offchain_cardano::event_sink::handler::LedgerCx;
        use spectrum_offchain::domain::event::{Channel, Confirmed, Transition};

        // Create a mock LedgerCx
        let ledger_context = LedgerCx {
            block_hash: BlockHeaderHash::from([0u8; 32]),
            slot: 100, // example slot number
                       // Add any necessary fields for LedgerCx initialization
        };

        // Create initial events
        let pool_init = Transition::Forward(Ior::Right(TestEvent::Pool { id: 1, init: true }));
        let event1 = Channel::Ledger(Confirmed(pool_init.clone()), ledger_context);
        let event2 = Channel::Ledger(
            Confirmed(Transition::Forward(Ior::Right(TestEvent::Order {
                id: 2,
                init: true,
            }))),
            ledger_context,
        );
        let cancel_event = Channel::Ledger(
            Confirmed(Transition::Backward(Ior::Left(TestEvent::Order {
                id: 2,
                init: true,
            }))),
            ledger_context,
        );

        // Initialize the SessionInProgress
        let mut session = SessionInProgress::new(pool_init, ledger_context, 130, 20);

        // Register event2
        assert!(session.register_event(event2.clone()).is_ok());
        assert_eq!(session.event_registry.len(), 2);

        // Register cancellation event for event2
        assert!(session.register_event(cancel_event.clone()).is_ok());
        assert_eq!(session.event_registry.len(), 1); // Event2 should be cancelled out

        // Upgrade the session with different slots
        let upgraded_events = session.upgrade(151); // Finalized slot
        assert!(upgraded_events.is_some());
        let upgraded_events = upgraded_events.unwrap();

        assert_eq!(
            upgraded_events.len(),
            1,
            "Only the opening event should remain in the upgraded events"
        );
        assert!(upgraded_events.contains(&event1));
    }

    #[test]
    fn events_ordering_on_finalization() {
        use super::SessionInProgress;
        use bloom_offchain_cardano::event_sink::handler::LedgerCx;
        use spectrum_offchain::domain::event::{Channel, Confirmed, Transition};

        // Create a mock LedgerCx
        let ledger_context_1 = LedgerCx {
            block_hash: BlockHeaderHash::from([0u8; 32]),
            slot: 100,
        };

        let ledger_context_2 = LedgerCx {
            block_hash: BlockHeaderHash::from([1u8; 32]),
            slot: 120,
        };

        let ledger_context_3 = LedgerCx {
            block_hash: BlockHeaderHash::from([1u8; 32]),
            slot: 140,
        };

        let expected_window_size = 12;

        // Create initial events
        let pool_init = Transition::Forward(Ior::Right(TestEvent::Pool { id: 1, init: true }));
        let event1 = Channel::Ledger(Confirmed(pool_init.clone()), ledger_context_1);

        // Initialize the SessionInProgress
        let mut session = SessionInProgress::new(pool_init, ledger_context_1, 140, 20);

        let mut rng = rand::thread_rng();

        for i in 1..=30 {
            let order_event = Channel::Ledger(
                Confirmed(Transition::Forward(Ior::Right(TestEvent::Order {
                    id: rng.next_u64() % 1_000_000,
                    init: true,
                }))),
                if i >= 10 {
                    ledger_context_2
                } else if i >= 20 {
                    ledger_context_3
                } else {
                    ledger_context_1
                },
            );

            // Register the generated order event
            assert!(session.register_event(order_event).is_ok(),);
        }

        // Upgrade the session with final slot
        let yielded_events = session
            .upgrade(160)
            .expect("Session events must be released at this point");
        let events_ordering = yielded_events.iter().map(|e| e.stable_id()).collect::<Vec<_>>();

        // Check that all events in events_ordering are lexicographically ordered within the single window
        assert!(
            events_ordering[0..expected_window_size]
                .into_iter()
                .fold((true, 0), |(acc, prev), x| (acc && prev <= *x, *x))
                .0,
            "Events should be ordered within session window"
        );

        assert_eq!(
            yielded_events.len(),
            31,
            "All events should remain in the yielded events"
        );
        assert!(yielded_events.contains(&event1));
    }

    #[test]
    fn test_session_upgrade_and_finalize() {
        use super::SessionInProgress;
        use bloom_offchain_cardano::event_sink::handler::LedgerCx;
        use spectrum_offchain::domain::event::{Channel, Confirmed, Transition};

        // Create a mock LedgerCx
        let ledger_context_1 = LedgerCx {
            block_hash: BlockHeaderHash::from([0u8; 32]),
            slot: 100, // Slot for event 1
        };

        let ledger_context_2 = LedgerCx {
            block_hash: BlockHeaderHash::from([1u8; 32]),
            slot: 110, // Slot for event 2
        };

        let ledger_context_3 = LedgerCx {
            block_hash: BlockHeaderHash::from([2u8; 32]),
            slot: 120, // Slot for event 3
        };

        // Create initial events. First event is always a pool.
        let pool_init = Transition::Forward(Ior::Right(TestEvent::Pool { id: 1, init: true }));
        let event1 = Channel::Ledger(Confirmed(pool_init.clone()), ledger_context_1);

        let event2 = Channel::Ledger(
            Confirmed(Transition::Forward(Ior::Right(TestEvent::Order {
                id: 2,
                init: true,
            }))),
            ledger_context_2,
        );

        let cancel_event2 = Channel::Ledger(
            Confirmed(Transition::Backward(Ior::Left(TestEvent::Order {
                id: 2,
                init: true,
            }))),
            ledger_context_3.clone(),
        );

        let event3 = Channel::Ledger(
            Confirmed(Transition::Forward(Ior::Right(TestEvent::Order {
                id: 3,
                init: true,
            }))),
            ledger_context_3.clone(),
        );

        // Initialize the SessionInProgress with the opening event (first event is a pool).
        let mut session = SessionInProgress::new(pool_init, ledger_context_1, 110, 20);

        // Register events
        assert!(
            session.register_event(event2.clone()).is_ok(),
            "Failed to register event2"
        );
        assert!(
            session.register_event(cancel_event2.clone()).is_ok(),
            "Failed to register cancellation for event2"
        );
        assert!(
            session.register_event(event3.clone()).is_ok(),
            "Failed to register event3"
        );

        // Verify event registry after application
        assert_eq!(
            session.event_registry.len(),
            2,
            "Only events 1 (pool) and 3 should remain after cancellations"
        );

        // Upgrade the session with a new slot and finalize
        let upgraded_events = session.upgrade(131); // Finalized slot
        assert!(upgraded_events.is_some(), "No events finalized");
        let upgraded_events = upgraded_events.unwrap();

        // Verify the finalized events
        assert_eq!(
            upgraded_events.len(),
            2,
            "Only events 1 (pool) and 3 should remain finalized after upgrade"
        );
        assert!(
            upgraded_events.contains(&event1),
            "Event1 (pool) should be in the finalized events"
        );
        assert!(
            upgraded_events.contains(&event3),
            "Event3 should be in the finalized events"
        );
    }
}
