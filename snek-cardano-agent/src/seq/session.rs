use bloom_offchain_cardano::event_sink::handler::LedgerCx;
use cml_core::Slot;
use cml_crypto::BlockHeaderHash;
use log::{trace, warn};
use spectrum_offchain::data::ior::Ior;
use spectrum_offchain::display::display_vec;
use spectrum_offchain::domain::event::{Channel, Confirmed, Transition};
use spectrum_offchain::domain::{SeqState, Stable};
use spectrum_offchain_cardano::raw_bytes::RawBytes;
use std::collections::hash_map::Entry;
use std::collections::{HashMap, VecDeque};
use std::fmt::Display;
use std::hash::Hash;

pub(crate) struct SessionInProgress<K, T> {
    opening_event: K,
    opening_event_followups: VecDeque<Channel<Transition<T>, LedgerCx>>,
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
            opening_event_followups: VecDeque::new(),
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
        K: Copy + Eq + Hash + Display,
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
                        // Confirmation of previously seen event
                        trace!("Registering initial event for entity: {}", event.stable_id());
                        self.confirmation_ordering.push_back((event_key, confirmed_at));
                        entry.insert(event);
                    } else if self.opening_event == event_key {
                        trace!("Registering follow-up for opening event: {}", event.stable_id());
                        self.opening_event_followups.push_back(event);
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
                    trace!("Registering subsequent event for entity: {}", event.stable_id());
                    entry.insert(event);
                } else {
                    warn!("Event {} is not registered", event_key,);
                }
            }
        }
        Ok(())
    }

    /// Clock upgrade may result in finalization of the session
    /// and accumulated events being released.
    pub(crate) fn upgrade(&mut self, slot: Slot) -> Option<Vec<Channel<Transition<T>, LedgerCx>>>
    where
        K: Copy + Eq + Ord + Hash + Display + RawBytes,
        T: Stable<StableId = K>,
    {
        if slot >= self.sealed_at + self.settlement_delay {
            let mut settled_events = vec![];
            let mut remaining_events = vec![];
            if let Some(event) = self.event_registry.remove(&self.opening_event) {
                settled_events.push(event);
            }
            while let Some(event) = self.opening_event_followups.pop_front() {
                settled_events.push(event);
            }
            let to_skip = settled_events.len();
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
            let max_window_size = settled_events.len() - to_skip;
            let window_size = seq_window_size(max_window_size, self.opening_event_cx.block_hash);
            trace!(
                "Total events sealed: {}, window size: {}",
                max_window_size, window_size
            );
            trace!(
                "Initial ordering: {}",
                display_vec(&settled_events.iter().map(|x| x.stable_id()).collect())
            );
            let reordered_events = do_sequencing(settled_events, to_skip, window_size);
            trace!(
                "Updated ordering: {}",
                display_vec(&reordered_events.iter().map(|x| x.stable_id()).collect())
            );

            return Some(
                reordered_events
                    .into_iter()
                    .chain(remaining_events.into_iter())
                    .collect(),
            );
        }
        None
    }
}

fn do_sequencing<T, K>(
    mut input: Vec<Channel<Transition<T>, LedgerCx>>,
    skip: usize,
    window_size: usize,
) -> Vec<Channel<Transition<T>, LedgerCx>>
where
    T: Stable<StableId = K>,
    K: RawBytes,
{
    let salt = key_to_int(input[(skip + window_size).saturating_sub(1)].stable_id());
    input[skip..skip + window_size]
        .sort_by(|a, b| (key_to_int(a.stable_id()) ^ salt).cmp(&(key_to_int(b.stable_id()) ^ salt)));
    input
}

fn key_to_int<K: RawBytes>(key: K) -> u64 {
    let bytes = key.to_raw_bytes();
    bytes.iter().fold(0u64, |acc, &byte| (acc << 8) | byte as u64)
}

// Determine sequencing window based on deterministic block data
fn seq_window_size(max_win_size: usize, block_hash: BlockHeaderHash) -> usize {
    if max_win_size != 0 {
        let max_cut_size = max_win_size / 4;
        let cut = if max_cut_size > 0 {
            key_to_int(block_hash) % (max_cut_size as u64)
        } else {
            0
        };
        max_win_size - cut as usize
    } else {
        0
    }
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
    use crate::seq::session::key_to_int;
    use bloom_offchain_cardano::event_sink::handler::LedgerCx;
    use cml_crypto::BlockHeaderHash;
    use rand::RngCore;
    use spectrum_offchain::data::ior::Ior;
    use spectrum_offchain::domain::event::{Channel, Confirmed, Transition};
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
    fn test_key_to_int_deterministic() {
        use super::key_to_int;

        // Define example keys
        let key1 = [0u8; 8]; // All zeros
        let key2 = [0xABu8; 8]; // All bytes set to 0xAB
        let key3 = [0xFFu8; 8]; // All bytes set to 0xFF
        let key4 = [0x01u8, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08]; // Sequential bytes

        // Use keys multiple times to ensure determinism
        let key1_result1: u64 = key_to_int(key1);
        let key1_result2: u64 = key_to_int(key1);
        let key2_result1: u64 = key_to_int(key2);
        let key2_result2: u64 = key_to_int(key2);
        let key3_result1: u64 = key_to_int(key3);
        let key3_result2: u64 = key_to_int(key3);
        let key4_result1: u64 = key_to_int(key4);
        let key4_result2: u64 = key_to_int(key4);

        // Ensure consistency in results
        assert_eq!(
            key1_result1, key1_result2,
            "Key1 should produce consistent results"
        );
        assert_eq!(
            key2_result1, key2_result2,
            "Key2 should produce consistent results"
        );
        assert_eq!(
            key3_result1, key3_result2,
            "Key3 should produce consistent results"
        );
        assert_eq!(
            key4_result1, key4_result2,
            "Key4 should produce consistent results"
        );
    }

    #[test]
    fn test_do_sequencing_deterministic() {
        use super::do_sequencing;

        let ledger_context = LedgerCx {
            block_hash: BlockHeaderHash::from([0u8; 32]),
            slot: 100, // example slot number
                       // Add any necessary fields for LedgerCx initialization
        };

        // Create a set of test events
        let event0 = TestEvent::Pool { id: 0, init: true };
        let event1 = TestEvent::Order { id: 1, init: true };
        let event2 = TestEvent::Order { id: 2, init: true };
        let event3 = TestEvent::Order { id: 3, init: true };
        let event4 = TestEvent::Order { id: 4, init: true };
        let event5 = TestEvent::Order { id: 5, init: true };

        let events_sequence = vec![event0, event3, event1, event4, event2, event5]
            .into_iter()
            .map(|e| Channel::Ledger(Confirmed(Transition::Forward(Ior::Right(e))), ledger_context))
            .collect::<Vec<_>>();

        // Perform sequencing on each sequence independently
        let result1 = do_sequencing(events_sequence.clone(), 1, 5);
        let result2 = do_sequencing(events_sequence, 1, 5);

        // Ensure all results are the same regardless of input order
        assert_eq!(
            result1, result2,
            "do_sequencing should be deterministic"
        );
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
            block_hash: BlockHeaderHash::from([9u8; 32]),
            slot: 100,
        };

        let ledger_context_2 = LedgerCx {
            block_hash: BlockHeaderHash::from([1u8; 32]),
            slot: 120,
        };

        let ledger_context_3 = LedgerCx {
            block_hash: BlockHeaderHash::from([2u8; 32]),
            slot: 140,
        };

        let expected_window_size = 27usize;

        // Create initial events
        let pool_init = Transition::Forward(Ior::Right(TestEvent::Pool { id: 1, init: true }));
        let event1 = Channel::Ledger(Confirmed(pool_init.clone()), ledger_context_1);

        // Initialize the SessionInProgress
        let mut session = SessionInProgress::new(pool_init, ledger_context_1, 140, 20);

        let mut rng = rand::thread_rng();

        let mut original_ordering = vec![event1.stable_id()];

        for i in 1..=30 {
            let order_event = Channel::Ledger(
                Confirmed(Transition::Forward(Ior::Right(TestEvent::Order {
                    id: rng.next_u64() % 1_000_000,
                    init: true,
                }))),
                if i >= 20 {
                    ledger_context_3
                } else if i >= 10 {
                    ledger_context_2
                } else {
                    ledger_context_1
                },
            );

            original_ordering.push(order_event.stable_id());
            // Register the generated order event
            assert!(session.register_event(order_event).is_ok(),);
        }

        // Upgrade the session with final slot
        let yielded_events = session
            .upgrade(160)
            .expect("Session events must be released at this point");
        let events_ordering = yielded_events.iter().map(|e| e.stable_id()).collect::<Vec<_>>();

        // Verify that events ordered properly within sequencing window
        let salt = key_to_int(original_ordering[(1 + expected_window_size).saturating_sub(1)]);
        assert!(
            events_ordering[1..expected_window_size]
                .into_iter()
                .map(|x| key_to_int(*x) ^ salt)
                .fold((true, 0u64), |(acc, prev), x| (acc && prev <= x, x))
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
