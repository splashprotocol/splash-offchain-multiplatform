use crate::seq::cond::Validations::{CancellationLock, HypedLaunch};
use crate::seq::cond::{ConditionalValidation, Id};
use bloom_offchain_cardano::event_sink::handler::LedgerCx;
use cml_core::Slot;
use cml_crypto::BlockHeaderHash;
use log::{info, trace, warn};
use spectrum_offchain::data::ior::Ior;
use spectrum_offchain::display::display_vec;
use spectrum_offchain::domain::event::{Channel, Confirmed, Transition};
use spectrum_offchain::domain::{SeqState, Stable};
use spectrum_offchain_cardano::raw_bytes::RawBytes;
use std::collections::hash_map::Entry;
use std::collections::{HashMap, VecDeque};
use std::fmt::Display;
use std::hash::{DefaultHasher, Hash, Hasher};

#[derive(Debug)]
pub(crate) struct SessionInProgress<K, T> {
    opening_event: K,
    opening_event_followups: VecDeque<Channel<Transition<T>, LedgerCx>>,
    opening_event_cx: LedgerCx,
    original_ordering: VecDeque<K>,
    confirmation_ordering: VecDeque<(K, Slot)>,
    event_registry: HashMap<K, Channel<Transition<T>, LedgerCx>>,
    /// Events that were collected before the session was triggered. Not being sequenced.
    premature_events: Vec<Channel<Transition<T>, LedgerCx>>,
    sealed_at: Slot,
    settlement_delay: Slot,
    capped: bool,
}

impl<K, T> SessionInProgress<K, T> {
    pub(crate) fn new(
        event: Transition<T>,
        premature_events: Vec<Channel<Transition<T>, LedgerCx>>,
        cx: LedgerCx,
        sealed_at: Slot,
        settlement_delay: Slot,
        capped: bool,
    ) -> Self
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
            premature_events,
            sealed_at,
            settlement_delay,
            capped,
        }
    }

    pub(crate) fn register_event(
        &mut self,
        event: Channel<Transition<T>, LedgerCx>,
    ) -> Result<(), SessionRejection>
    where
        K: Copy + Eq + Hash + Display,
        T: Stable<StableId = K>
            + ConditionalValidation<{ HypedLaunch as u8 }, ()>
            + ConditionalValidation<{ CancellationLock as u8 }, Slot>,
    {
        let event_key = event.stable_id();
        if self.capped && !event.is_valid(Id, ()) {
            trace!("Event {}, buy cap is invalid", event_key,);
            return Ok(());
        }
        if !event.is_valid(Id, self.sealed_at + self.settlement_delay) {
            trace!("Event {}, cancellation delay too small", event_key,);
            return Ok(());
        }
        match self.event_registry.entry(event_key) {
            Entry::Occupied(mut entry) => {
                let current = entry.get();
                if is_cancellation(&event) {
                    self.original_ordering.retain(|k| *k != event_key);
                    entry.remove();
                    if event_key == self.opening_event {
                        info!("Session for opening event {} is cancelled", event_key,);
                        return Err(SessionRejection::SessionCancelled);
                    }
                } else {
                    if let Some(confirmed_at) = is_confirmation(current, &event) {
                        // Confirmation of a previously seen event
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
                    warn!("Unknown cancellation event {}", event_key,);
                    self.premature_events.push(event);
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
            let mut remaining_events = self.premature_events.drain(..).collect::<Vec<_>>();
            // Remove events that have been confirmed
            remaining_events.retain(|k| self.event_registry.get(&k.stable_id()).is_none());
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
                max_window_size,
                window_size
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
            trace!(
                "Remaining events: {}",
                display_vec(&remaining_events.iter().map(|x| x.stable_id()).collect())
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

#[derive(Debug)]
pub enum SessionRejection {
    SessionCancelled,
}

fn do_sequencing<T, K>(
    mut input: Vec<Channel<Transition<T>, LedgerCx>>,
    skip: usize,
    window_size: usize,
) -> Vec<Channel<Transition<T>, LedgerCx>>
where
    T: Stable<StableId = K>,
    K: Copy + RawBytes,
{
    let salt = input[(skip + window_size).saturating_sub(1)].stable_id();
    input[skip..skip + window_size]
        .sort_by(|a, b| seq_key(a.stable_id(), salt).cmp(&seq_key(b.stable_id(), salt)));
    input
}

fn seq_key<K: RawBytes>(key: K, salt: K) -> u64 {
    let raw_key = key.to_raw_bytes();
    let raw_salt = salt.to_raw_bytes();
    let mut hasher = DefaultHasher::new();
    raw_key.hash(&mut hasher);
    raw_salt.hash(&mut hasher);
    hasher.finish()
}

fn key_to_int<K: Hash>(key: K) -> u64 {
    let mut hasher = DefaultHasher::new();
    key.hash(&mut hasher);
    hasher.finish()
}

// Determine the sequencing window based on deterministic block data
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
    use crate::seq::cond::{ConditionalValidation, Id, Validations};
    use crate::seq::session::{do_sequencing, seq_key};
    use bloom_offchain_cardano::event_sink::handler::LedgerCx;
    use cml_crypto::BlockHeaderHash;
    use rand::RngCore;
    use spectrum_cardano_lib::Token;
    use spectrum_offchain::data::ior::Ior;
    use spectrum_offchain::display::display_vec;
    use spectrum_offchain::domain::event::{Channel, Confirmed, Transition};
    use spectrum_offchain::domain::{SeqState, Stable};

    struct Ev {
        id: Token,
        init: bool,
    }

    impl Stable for Ev {
        type StableId = Token;

        fn stable_id(&self) -> Self::StableId {
            self.id
        }

        fn is_quasi_permanent(&self) -> bool {
            false
        }
    }

    impl SeqState for Ev {
        fn is_initial(&self) -> bool {
            self.init
        }
    }

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

    impl<Cx> ConditionalValidation<{ Validations::HypedLaunch as u8 }, Cx> for TestEvent {
        fn cond(&self, _: Id<{ Validations::HypedLaunch as u8 }>) -> bool {
            false
        }

        fn is_valid(&self, _: Id<{ Validations::HypedLaunch as u8 }>, _: Cx) -> bool {
            false
        }
    }

    impl<Cx> ConditionalValidation<{ Validations::CancellationLock as u8 }, Cx> for TestEvent {
        fn cond(&self, _: Id<{ Validations::CancellationLock as u8 }>) -> bool {
            true
        }

        fn is_valid(&self, _: Id<{ Validations::CancellationLock as u8 }>, _: Cx) -> bool {
            true
        }
    }

    #[test]
    fn test_do_sequencing() {
        let ledger_context = LedgerCx {
            block_hash: BlockHeaderHash::from([0u8; 32]),
            slot: 100, // example slot number
                       // Add any necessary fields for LedgerCx initialization
        };
        let raw_ids = vec![
            "63f947b8d9535bc4e4ce6919e3dc056547e8d30ada12f29aa5f826b8.ed375f4db261ca9d22949d433183aafdeb773e117466cfbdb6a6f29d03e7d05e",
            "29c2c5760b8ebf9ecec8954d800b928b91ce0aea9881bd13f886a5cf.",
            "a7c243562e714dfcc2524cbb6a296daace69199697a15013ee91bb33.",
            "fa2051580d1639012cc7acc2c6487b7ece2d653118358412183f4aae.",
            "63e9ed4170eb1a8713858481c09843e8021d6dfa662fdbe4b7ce53a2.",
            "98a5169a516c52a9001feaabeb2c979acefffb4b19dd4ef924c5d348.",
            "9d779bca6853a435f2b055ea07173e8b184f9f4a5aeea0224c54270d.",
            "6b8bfa90fd17ccf29c45d3fe36e3ae8b83c50ed5c11307fb9c237416.",
            "155155ad0247f40051475f9441dfa09a81bbe785c24f9986ef00ac50.",
            "fbfc6a74e4667c3b467eadb2210b0a80f2308fbcbd8a62e70fd055c9.",
            "378f54110c9e0f5414e3a6b12b22b1165f9c8bc69ff8badf2aea550d.",
            "709cf120bb3943d6d2f6a6a6e48b301a8fa25d6cc5b5559760623fbc.",
            "708367cb738024a13c59fef7d2fa0ce29ca08ea2fb433851331fc4d6.",
            "41472275619a632a89223d35f98687d63a8ceaeff0990cbfe605090f.",
            "7507d5829183bb6567282f54aac7e40eb0b1aebb8fd21d2036a23f58.",
            "faa114e5c56e7e304c1c9fecbe9e39c050357d7165c1995837a50d25.",
            "74e28a8bc69f3a264f2e20c49d3db5083e67c1dcb25d60af8dfbd616.",
            "c0626432ce882b68f438973043613f7c7e4c37df5bc0cfb35d35fc80.",
            "47cf15cdad313ede6acf8ff1a6fe552430c4f07a0aa3b884883c21e8.",
            "347d18837b41470b099c565a299fb36543a205c25a1561b3737ed807.",
            "8c63952bdb0748f700d2c2b82508e929f93b0c6db87eef1a920d8ee8.",
            "dfdebeca11d9d4477994c8b9a8c87009b27747a08561eba226c4f060.",
            "835cce927448203cbee812e32c0831227ffb2735a827d407be3361b7.",
            "bfaff6340c624aff28c4b2d4d068fec08205f0153b83ee66daed274b.",
            "6d25b761046b051dd5a89c5c93022b80826c203493543eccd32dc6b9.",
            "f336fc8a9e28d139f565a178c5195e957d7339c74c065cf5aa79dede.",
            "365fb57d6fffdec1c86517c03506866b32d88a4ae923ec1023f7adf5.",
            "b1177875facdee3705320f2f870118af0153527eea4894978257c4e4.",
            "fce10d72e2ae15f0a1ef128d84147fec9d82ff832e0ecd541498cd5c.",
            "2fcc1c514cd6f959edbb8962d599b14fc9e4309f5926d726ab5208f3.",
            "be894b73a1935777d118b1dae641257991b92da665d25c975b46db7e.",
            "6a8c54e1038a1aec04d221cc02829c08d3fa41aa672173cd6a026a7b.",
            "b1a31783f54486da21db9a5d999fd09bc8381d70820a387304c697ea.",
            "95b4763601f964ac84b1138d7358ed6a246b8499a742453b889a8ef1.",
            "1d430c4d8ce1cdc9af79c5961fc55156f62995289dc88f5766ff8314.",
            "9fbf06f58ecdc66567f91ba6bdc5dfb3f9ba6f2637ac433931f8be74.",
            "97f9eea517b442da50405b0a31277d419dc0298d67b5ac6d7c891404.",
            "639eb59c6c27261d308b215ec6454b879a61b5b0bfc7ad0e0f1da1f2.",
            "bae90e6a4273bc433bc6f2815d3897bfad51c597e6546a3c70afbfd9.",
        ];
        let orders = raw_ids
            .into_iter()
            .map(Token::from_string_unsafe)
            .map(|id| Ev { id, init: true })
            .map(|e| Channel::Ledger(Confirmed(Transition::Forward(Ior::Right(e))), ledger_context))
            .collect();
        let result1 = do_sequencing(orders, 1, 33);
        println!(
            "Ord: {}",
            display_vec(&result1.iter().map(|x| x.stable_id()).collect())
        )
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
    fn test_seq_key_with_salt_deterministic() {
        use super::seq_key;

        // Define example sequences and salts (both arguments must have the same type)
        let seq1: [u8; 16] = [0u8; 16]; // All zeros
        let seq2: [u8; 16] = [0xABu8; 16]; // All bytes set to 0xAB
        let seq3: [u8; 16] = [0xFFu8; 16]; // All bytes set to 0xFF
        let seq4: [u8; 16] = [
            0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0A, 0x0B, 0x0C, 0x0D, 0x0E, 0x0F, 0x10,
        ]; // Sequential bytes

        let salt1: [u8; 16] = [0u8; 16]; // Salt all zeros
        let salt2: [u8; 16] = [0xFFu8; 16]; // Salt all 0xFF

        // Use sequences with salts multiple times to ensure determinism
        let seq1_result1: u64 = seq_key(seq1, salt1);
        let seq1_result2: u64 = seq_key(seq1, salt1);
        let seq2_result1: u64 = seq_key(seq2, salt2);
        let seq2_result2: u64 = seq_key(seq2, salt2);
        let seq3_result1: u64 = seq_key(seq3, salt1);
        let seq3_result2: u64 = seq_key(seq3, salt1);
        let seq4_result1: u64 = seq_key(seq4, salt2);
        let seq4_result2: u64 = seq_key(seq4, salt2);

        // Ensure consistency in results
        assert_eq!(
            seq1_result1, seq1_result2,
            "Seq1 with salt1 should produce consistent results"
        );
        assert_eq!(
            seq2_result1, seq2_result2,
            "Seq2 with salt2 should produce consistent results"
        );
        assert_eq!(
            seq3_result1, seq3_result2,
            "Seq3 with salt1 should produce consistent results"
        );
        assert_eq!(
            seq4_result1, seq4_result2,
            "Seq4 with salt2 should produce consistent results"
        );

        // Further ensure different salt affects result
        let seq1_salt2_result = seq_key(seq1, salt2);
        assert_ne!(
            seq1_result1, seq1_salt2_result,
            "Seq1 results should differ with different salts"
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
        assert_eq!(result1, result2, "do_sequencing should be deterministic");
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

        let premature_events = Vec::new();

        // Initialize the SessionInProgress
        let mut session = SessionInProgress::new(pool_init, premature_events, ledger_context, 130, 20, false);

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

        let expected_window_size = 25usize;

        // Create initial events
        let pool_init = Transition::Forward(Ior::Right(TestEvent::Pool { id: 1, init: true }));
        let event1 = Channel::Ledger(Confirmed(pool_init.clone()), ledger_context_1);

        let premature_events = Vec::new();
        // Initialize the SessionInProgress
        let mut session = SessionInProgress::new(pool_init, premature_events, ledger_context_1, 140, 20, false);

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
        let salt = original_ordering[(1 + expected_window_size).saturating_sub(1)];
        assert!(
            events_ordering[1..expected_window_size]
                .into_iter()
                .map(|x| seq_key(*x, salt))
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

        // Create initial events. The first event is always a pool.
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

        let premature_events = Vec::new();
        // Initialize the SessionInProgress with the opening event (first event is a pool).
        let mut session = SessionInProgress::new(pool_init, premature_events, ledger_context_1, 110, 20, false);

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
