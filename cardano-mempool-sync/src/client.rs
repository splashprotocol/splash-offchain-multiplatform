use std::collections::HashSet;
use std::future::Future;
use std::marker::PhantomData;
use std::path::Path;
use std::sync::Arc;

use async_stream::stream;
use cml_core::serialization::Deserialize;
use cml_crypto::blake2b256;
use futures::Stream;
use log::{info, warn};
use pallas_codec::minicbor::Decoder;
use pallas_network::miniprotocols::{handshake, txmonitor, PROTOCOL_N2C_HANDSHAKE};
use pallas_network::multiplexer;
use pallas_network::multiplexer::{Bearer, RunningPlexer};
use tokio::sync::{mpsc, Mutex};

pub struct LocalTxMonitorClient<Tx> {
    plexer: RunningPlexer,
    tx_monitor: Arc<Mutex<MonitorState>>,
    /// The sender side of the forget channel, kept outside the monitor mutex.
    forget: ForgetHandle,
    tx: PhantomData<Tx>,
}

impl<Tx: Send + Sync> LocalTxMonitorClient<Tx> {
    #[cfg(not(target_os = "windows"))]
    pub async fn connect(path: impl AsRef<Path>, magic: u64) -> Result<Self, Error> {
        let bearer = Bearer::connect_unix(path).await.map_err(Error::ConnectFailure)?;

        let mut mplex = multiplexer::Plexer::new(bearer);

        let hs_channel = mplex.subscribe_client(PROTOCOL_N2C_HANDSHAKE);
        let tm_channel = mplex.subscribe_client(PROTOCOL_N2C_TX_MONITOR);

        let plexer = mplex.spawn();

        let versions = handshake::n2c::VersionTable::v10_and_above(magic);
        let mut client = handshake::Client::new(hs_channel);

        let handshake = client
            .handshake(versions)
            .await
            .map_err(Error::HandshakeProtocol)?;

        if let handshake::Confirmation::Rejected(_reason) = handshake {
            return Err(Error::IncompatibleVersion);
        }

        let dedup = SnapshotDedup::new(FILTER_CAP);
        let forget = dedup.forget_handle();
        let state = MonitorState {
            client: txmonitor::Client::new(tm_channel),
            dedup,
        };

        Ok(Self {
            plexer,
            tx_monitor: Arc::new(Mutex::new(state)),
            forget,
            tx: PhantomData::default(),
        })
    }

    /// A handle that makes the monitor forget a tx id, so the next snapshot
    /// that lists the tx yields it again. Take it before
    /// [`Self::stream_updates`] consumes the client. It does not take the
    /// monitor mutex.
    pub fn forget_handle(&self) -> ForgetHandle {
        self.forget.clone()
    }

    pub fn stream_updates<'a>(self) -> impl Stream<Item = Tx> + Send + 'a
    where
        Tx: Deserialize + 'a,
    {
        stream! {
            let mut seq_num = 0;
            loop {
                let mut guard = self.tx_monitor.lock().await;
                let MonitorState { client, dedup } = &mut *guard;
                // Forgets sent before AND during the wait for the snapshot
                // apply to it, so a forgotten tx is yielded on its first
                // re-list, not one snapshot late.
                if dedup.around_acquire(client.acquire()).await.is_ok() {
                    while let Ok(Some(raw_tx)) = client.query_next_tx().await {
                        let bytes = &*raw_tx.1;
                        if dedup.admit(bytes, seq_num) {
                            if let Some(tx) = Tx::from_cbor_bytes(bytes).ok() {
                                yield tx;
                            }
                        }
                    }
                }
                seq_num += 1;
            }
        }
    }

    pub async fn close(self) {
        self.plexer.abort().await
    }
}

const PROTOCOL_N2C_TX_MONITOR: u16 = 9;
const FILTER_CAP: usize = 16384;

/// The real transaction id: blake2b-256 of the tx body bytes.
pub type TxId = [u8; 32];

/// The filter key of a raw mempool tx: its real id, read off the bytes.
///
/// The tx is the CBOR array `[body, witnesses, is_valid, aux]`; the key is
/// blake2b-256 of the body exactly as it is encoded, which is the tx id.
/// Bytes that are not a CBOR array with a whole first element are keyed by
/// blake2b-256 of all the bytes, so every tx still has a key.
pub fn tx_id_of_raw(bytes: &[u8]) -> TxId {
    match body_span(bytes) {
        Some(body) => blake2b256(body),
        None => blake2b256(bytes),
    }
}

/// Lower-case hex of a tx id, for the log.
fn hex(id: &TxId) -> String {
    id.iter().map(|b| format!("{b:02x}")).collect()
}

fn body_span(bytes: &[u8]) -> Option<&[u8]> {
    let mut decoder = Decoder::new(bytes);
    // A definite or an indefinite array; its first element is the body.
    decoder.array().ok()?;
    let start = decoder.position();
    decoder.skip().ok()?;
    let end = decoder.position();
    bytes.get(start..end)
}

/// A cloneable handle that tells the monitor to forget a tx id. `forget`
/// only sends on an unbounded channel: it never awaits and never takes the
/// monitor mutex. The monitor applies the forgets before and right after the
/// `acquire` of its next snapshot, so the next snapshot is the one that
/// yields the tx again.
#[derive(Clone)]
pub struct ForgetHandle(mpsc::UnboundedSender<TxId>);

impl ForgetHandle {
    /// Send `id` to the monitor and return. When the monitor is gone the
    /// forget is dropped, and the log says so.
    pub fn forget(&self, id: TxId) {
        match self.0.send(id) {
            Ok(()) => info!("[MEMPOOL_FORGET] queued tx={}", hex(&id)),
            Err(_) => warn!("[MEMPOOL_FORGET] dropped tx={} — the monitor is gone", hex(&id)),
        }
    }
}

/// The per-snapshot dedup of the mempool monitor: the projection of tx ids
/// already yielded and the receiver of the forgets.
///
/// `stream_updates` calls [`Self::around_acquire`] for each `acquire` (it
/// drains the forgets before and right after the wait) and [`Self::admit`]
/// for each tx the snapshot lists.
pub struct SnapshotDedup {
    mempool: MempoolProjection,
    forget_rx: mpsc::UnboundedReceiver<TxId>,
    forget_tx: mpsc::UnboundedSender<TxId>,
    /// The ids forgotten for the current snapshot whose re-yield is not
    /// logged yet. Reset by [`Self::around_acquire`], so a forgotten tx the
    /// snapshot does not list leaves no trace behind.
    forgotten: HashSet<TxId>,
}

impl SnapshotDedup {
    pub fn new(cap: usize) -> Self {
        let (forget_tx, forget_rx) = mpsc::unbounded_channel();
        Self {
            mempool: MempoolProjection::new(cap),
            forget_rx,
            forget_tx,
            forgotten: HashSet::new(),
        }
    }

    /// A handle that sends forgets to this dedup.
    pub fn forget_handle(&self) -> ForgetHandle {
        ForgetHandle(self.forget_tx.clone())
    }

    /// The dedup side of one snapshot: apply the forgets sent so far, run
    /// `acquire` (the wait for the next snapshot), then apply the forgets
    /// that arrived during the wait, so they count for THIS snapshot and
    /// not the next one. Both drains are synchronous; the only await is
    /// `acquire` itself.
    pub async fn around_acquire<F: Future>(&mut self, acquire: F) -> F::Output {
        self.forgotten.clear();
        self.sync_forgets();
        let acquired = acquire.await;
        self.sync_forgets();
        acquired
    }

    /// Apply every forget sent so far. Returns how many were applied.
    pub fn sync_forgets(&mut self) -> usize {
        let mut applied = 0;
        while let Ok(id) = self.forget_rx.try_recv() {
            if self.mempool.forget(&id) {
                self.forgotten.insert(id);
            }
            info!("[MEMPOOL_FORGET] applied tx={}", hex(&id));
            applied += 1;
        }
        applied
    }

    /// Register the raw tx under the snapshot counter `seq_num`. `true` when
    /// the projection does not know it, so the snapshot yields it; `false`
    /// for a re-list.
    pub fn admit(&mut self, raw_tx: &[u8], seq_num: u64) -> bool {
        let id = tx_id_of_raw(raw_tx);
        let yielded = !self.mempool.register(id, seq_num);
        if yielded && self.forgotten.remove(&id) {
            info!("[MEMPOOL_FORGET] re-yielded tx={}", hex(&id));
        }
        yielded
    }
}

/// Two projections of the mempool, keyed by tx id. When the snapshot
/// counter advances, the current projection becomes the previous one and a
/// new current projection starts empty. A tx is a duplicate when either
/// projection holds it; a duplicate found in the previous projection is not
/// copied forward, so a tx listed in every snapshot is yielded again two
/// snapshots after its last yield.
struct MempoolProjection {
    prev_projection: HashSet<TxId>,
    current_projection: HashSet<TxId>,
    slot: u64,
}

impl MempoolProjection {
    fn new(capacity: usize) -> Self {
        Self {
            prev_projection: HashSet::with_capacity(capacity),
            current_projection: HashSet::with_capacity(capacity),
            slot: 0,
        }
    }

    /// Rotate the projections when `slot` advanced, then register `tx`.
    /// `true` when `tx` is already known (a duplicate), `false` when it is
    /// new and now registered in the current projection.
    fn register(&mut self, tx: TxId, slot: u64) -> bool {
        if slot > self.slot {
            self.prev_projection = std::mem::take(&mut self.current_projection);
            self.slot = slot;
        }

        if self.prev_projection.contains(&tx) || self.current_projection.contains(&tx) {
            true
        } else {
            self.current_projection.insert(tx);
            false
        }
    }

    /// Remove `tx` from both projections. `true` when either knew it.
    fn forget(&mut self, tx: &TxId) -> bool {
        let in_prev = self.prev_projection.remove(tx);
        let in_current = self.current_projection.remove(tx);
        in_prev || in_current
    }
}

struct MonitorState<C = txmonitor::Client> {
    client: C,
    dedup: SnapshotDedup,
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("error connecting bearer")]
    ConnectFailure(#[source] tokio::io::Error),

    #[error("handshake protocol error")]
    HandshakeProtocol(handshake::Error),

    #[error("handshake version not accepted")]
    IncompatibleVersion,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_tx_hash(byte: u8) -> TxId {
        [byte; 32]
    }

    #[test]
    fn test_mempool_projection_tx_lifecycle() {
        let mut projection = MempoolProjection::new(100);
        let tx1 = make_tx_hash(1);
        let tx2 = make_tx_hash(2);
        let tx3 = make_tx_hash(3);

        // Slot 1: tx1 is added to mempool, should return false (new tx)
        assert_eq!(projection.register(tx1, 1), false, "tx1 at slot 1 should be new");
        assert_eq!(projection.slot, 1);
        assert!(projection.current_projection.contains(&tx1));
        assert!(projection.prev_projection.is_empty());

        // Slot 2: tx1 is seen again, should return true (duplicate from previous slot)
        assert_eq!(
            projection.register(tx1, 2),
            true,
            "tx1 at slot 2 should be duplicate"
        );
        assert_eq!(projection.slot, 2);
        // tx1 should now be in prev_projection
        assert!(projection.prev_projection.contains(&tx1));
        // And should NOT be in current_projection since it returned true
        assert!(!projection.current_projection.contains(&tx1));

        // Slot 3: other txs added, but tx1 is not there
        assert_eq!(projection.register(tx2, 3), false, "tx2 at slot 3 should be new");
        assert_eq!(projection.register(tx3, 3), false, "tx3 at slot 3 should be new");
        assert_eq!(projection.slot, 3);
        // After slot advance, tx1 is no longer in prev_projection (it was in current at slot 2)
        assert!(!projection.prev_projection.contains(&tx1));
        assert!(projection.current_projection.contains(&tx2));
        assert!(projection.current_projection.contains(&tx3));

        // Slot 4: tx1 appears again, should return false (reappeared after being absent)
        assert_eq!(
            projection.register(tx1, 4),
            false,
            "tx1 at slot 4 should be new (reappeared)"
        );
        assert_eq!(projection.slot, 4);
        // tx1 is not in prev_projection (which had tx2, tx3 from slot 3)
        assert!(projection.prev_projection.contains(&tx2));
        assert!(projection.prev_projection.contains(&tx3));
        // tx1 is now in current_projection
        assert!(projection.current_projection.contains(&tx1));
    }

    #[test]
    fn test_mempool_projection_same_slot_multiple_txs() {
        let mut projection = MempoolProjection::new(100);
        let tx1 = make_tx_hash(1);
        let tx2 = make_tx_hash(2);

        // Multiple txs in same slot
        assert_eq!(projection.register(tx1, 1), false);
        assert_eq!(projection.register(tx2, 1), false);

        // Duplicate in same slot should return true (not in prev_projection)
        assert_eq!(projection.register(tx1, 1), true);

        // All txs should be in current_projection
        assert!(projection.current_projection.contains(&tx1));
        assert!(projection.current_projection.contains(&tx2));
    }

    #[test]
    fn test_mempool_projection_slot_skip() {
        let mut projection = MempoolProjection::new(100);
        let tx1 = make_tx_hash(1);

        // Add tx1 at slot 1
        assert_eq!(projection.register(tx1, 1), false);

        // Skip to slot 5 (simulate missing slots)
        let tx2 = make_tx_hash(2);
        assert_eq!(projection.register(tx2, 5), false);
        assert_eq!(projection.slot, 5);

        // tx1 should be in prev_projection
        assert!(projection.prev_projection.contains(&tx1));
        assert!(projection.current_projection.contains(&tx2));
    }

    /// The rotation rule of `register`, for a tx that every snapshot lists:
    /// seen in `n` it is registered and yielded; in `n+1` it is found in the
    /// previous projection, suppressed and NOT copied into the current one;
    /// in `n+2` neither projection holds it, so it is yielded again. The
    /// forget path must leave this rule as it is.
    #[test]
    fn a_tx_listed_in_every_snapshot_is_yielded_again_two_snapshots_later() {
        let mut projection = MempoolProjection::new(100);
        let tx = make_tx_hash(1);
        for n in (1..=9).step_by(2) {
            assert!(!projection.register(tx, n), "seq {n}: yielded");
            assert!(
                projection.register(tx, n),
                "seq {n}: a re-list in the same snapshot is suppressed"
            );
            assert!(
                projection.register(tx, n + 1),
                "seq {}: suppressed by the previous projection",
                n + 1
            );
            assert!(projection.prev_projection.contains(&tx));
            assert!(
                !projection.current_projection.contains(&tx),
                "a duplicate is not copied forward"
            );
        }
    }

    // ---------------------------------------------------------------------
    // The filter key is the real tx id
    // ---------------------------------------------------------------------

    /// A mainnet tx (copied from `cardano-utxo-monitor/src/index.rs`
    /// `TX_PRODUCE`). Its id is known from the chain: the output
    /// `13de3390…5afc#0` that `TX_CONSUME` spends is this tx's output.
    const FIXTURE_TX: &str = "84a7008182582086ecf8a72d7d1744deefc1c923c7f1ed8eb09a549cb70fde3d572316e066bfa403018182583901bed3c3bac9ddc7952cc91cf76db3dd808f99f4a0dd07e78e06657bc21cc69f513f9551f517c5212855ece1b34d0128f9f9b54e47cebadd4b821b0000000103f98686ad581c0ece814aa1cc2c98981c7690083dbcb51c5bb1279ae408873d8c8762a15820595479793659676e546b3069574c396a4544315943352f49516873337252343701581c15509d4cb60f066ca4c7e982d764d6ceb4324cb33776d1711da1beeea24e42616279416c69656e3034373231014e42616279416c69656e303831313801581c279c909f348e533da5808898f87f9a14bb2c3dfbbacccd631d927a3fa144534e454b19a839581c29d222ce763455e3d7a09a665ce554f00ac89d2e99a1a83d267170c6a1434d494e194a31581c51a5e236c4de3af2b8020442e2a26f454fda3b04cb621c1294a0ef34a144424f4f4b1a0165f8a0581c530a197fe7c275f204c3396b3782fc738f4968f0c81dd2291cf07b8aa3581a434330303337303030303030303030303030303135323030303001581a434330303337303030303030303030303030303135323030363401581a434330313531303030303030303030303030303137363030363401581c5ee425062d88069b702a38a357895132b9b50c8f893c8cf87a4c8c32a14445574d5401581ca0028f350aaabe0545fdcb56b039bfb08e4bb4d8c4d7c3c7d481c235a145484f534b591a04277dbf581ca7904896a247d3aa09478e856769b82d1f2e060028b6bda5543b699fa64d4343434f4c4c41423030303337014d4343434f4c4c41423030313531014d4343434f4c4c41423038393235014d4343434f4c4c4142303930333901581c4375746543726561747572657343686164694e61737361723030333701581c4375746543726561747572657343686164694e61737361723031353101581ce5a42a1a1d3d1da71b0449663c32798725888d2eb0843c4dabeca05aa151576f726c644d6f62696c65546f6b656e581a000f4240581cecbe846aa1a535579d67f9480fa6173b64d7e239df0460eba36e3ad0a14a0014df1053617475726e1a000f4240581cf0ff48bbb7bbe9d59a40f1ce90e9e9d0ff5002ec48f232b49ca0fb9aa14b736f667462696e61746f7201581cfe38ef97888dfde0292b7d2ed103543ecf92a419a29634f513a1d71fa14541534e454b1a00249f00021a00033cd9031a0936d6fe048183028200581c1cc69f513f9551f517c5212855ece1b34d0128f9f9b54e47cebadd4b581c538299a358e79a289c8de779f8cd09dd6a6bb286de717d1f744bb35705a1581de11cc69f513f9551f517c5212855ece1b34d0128f9f9b54e47cebadd4b1a002bdbc50758201c45c96126112c50a121c7bce6fa0b0fb8e7fa6990d0a9435a89a91a1460fccea1008282582061b2624741ddbcd41a6e3490b2e4a71bcc1cb6ff891137097540ad7fa8cf56115840a00c437ed8a787f4fbdd39ada04c3cf6191ee26abe44137a6eb4a2dcf55e4387bfbb565133d42df61f0ed45de43adeec019c2f8c5e0fae209534a266755b96038258201b775e64b1cb83f9420829d807b892da0c8c2ea89873028c620b8c04eb35ee4558400d97db78925083b9736b66ccf28593638cba828f5806311eb6d4aa7d861a53023a2ab27a5f56d65beafb177f756e746f98e093a8c56c5982667e4f2f5abf8403f5a11902a2a1636d736781781956455350523a20506172746e65722044656c65676174696f6e";
    const FIXTURE_TX_ID: &str = "13de3390f33b18faaeeb91eafc839e28c687f47f146e9c68779562a8a5385afc";

    fn unhex(s: &str) -> Vec<u8> {
        (0..s.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&s[i..i + 2], 16).expect("hex"))
            .collect()
    }

    /// A tx-shaped CBOR array `[body, witnesses, true, null]` whose body is
    /// the definite map `{2: n}`; distinct `n` give distinct ids.
    fn raw_tx(n: u8) -> Vec<u8> {
        vec![0x84, 0xa1, 0x02, 0x18, n, 0xa0, 0xf5, 0xf6]
    }

    /// One scripted snapshot with counter `seq_num` and without an `acquire`
    /// wait: the forgets sent so far are applied, then every listed tx is
    /// registered. Returns the positions of the txs the snapshot yields. The
    /// `acquire` side is `around_acquire`, exercised by its own tests.
    fn snapshot(dedup: &mut SnapshotDedup, seq_num: u64, txs: &[&[u8]]) -> Vec<usize> {
        dedup.sync_forgets();
        txs.iter()
            .enumerate()
            .filter(|(_, raw)| dedup.admit(raw, seq_num))
            .map(|(i, _)| i)
            .collect()
    }

    #[test]
    fn the_filter_key_is_the_real_tx_id() {
        let raw = unhex(FIXTURE_TX);
        assert_eq!(tx_id_of_raw(&raw).to_vec(), unhex(FIXTURE_TX_ID));
    }

    #[test]
    fn the_id_ignores_the_witnesses() {
        // Same body, other witness set: one tx id, so one filter key.
        let a = raw_tx(7);
        let mut b = raw_tx(7);
        b[5] = 0xa1;
        b.splice(6..6, [0x00, 0x80]);
        assert_ne!(a, b);
        assert_eq!(tx_id_of_raw(&a), tx_id_of_raw(&b));
        assert_eq!(tx_id_of_raw(&a), blake2b256(&[0xa1, 0x02, 0x18, 7]));
    }

    #[test]
    fn an_indefinite_outer_array_still_keys_on_the_body() {
        let definite = raw_tx(9);
        let mut indefinite = definite.clone();
        indefinite[0] = 0x9f;
        indefinite.push(0xff);
        assert_eq!(tx_id_of_raw(&indefinite), tx_id_of_raw(&definite));
    }

    #[test]
    fn bytes_that_are_not_a_cbor_array_keep_a_whole_bytes_key() {
        for raw in [vec![0x01u8, 0x02, 0x03], vec![], vec![0x84, 0xa1]] {
            assert_eq!(tx_id_of_raw(&raw), blake2b256(&raw), "{raw:02x?}");
        }
    }

    // ---------------------------------------------------------------------
    // Forget: the tx comes back on the next snapshot that lists it
    // ---------------------------------------------------------------------

    /// The forget, not the rotation, brings the tx back: `other` is seen
    /// alongside `t` in every snapshot and stays suppressed where `t`
    /// returns.
    #[test]
    fn a_forgotten_tx_is_yielded_again_on_the_next_snapshot() {
        let mut dedup = SnapshotDedup::new(FILTER_CAP);
        let forget = dedup.forget_handle();
        let t = raw_tx(1);
        let other = raw_tx(2);
        assert_eq!(snapshot(&mut dedup, 1, &[&t, &other]), vec![0, 1], "first sight yields both");
        assert_eq!(
            snapshot(&mut dedup, 2, &[&t, &other]),
            Vec::<usize>::new(),
            "a re-list is suppressed"
        );

        forget.forget(tx_id_of_raw(&t));
        assert_eq!(
            snapshot(&mut dedup, 2, &[&t, &other]),
            vec![0],
            "only the forgotten tx comes back"
        );
        assert_eq!(snapshot(&mut dedup, 2, &[&t, &other]), Vec::<usize>::new(), "and once only");
    }

    /// The forget reaches a tx in either projection: one registered in the
    /// current snapshot, and one rotated into the previous projection.
    #[test]
    fn a_forget_clears_both_projections() {
        let mut projection = MempoolProjection::new(100);
        let in_prev = make_tx_hash(1);
        let in_current = make_tx_hash(2);
        assert!(!projection.register(in_prev, 1));
        assert!(!projection.register(in_current, 2));
        assert!(projection.prev_projection.contains(&in_prev));
        assert!(projection.current_projection.contains(&in_current));

        assert!(projection.forget(&in_prev), "known in the previous projection");
        assert!(projection.forget(&in_current), "known in the current projection");
        assert!(!projection.forget(&in_prev), "forgotten already");
        assert!(projection.prev_projection.is_empty());
        assert!(projection.current_projection.is_empty());

        // Both register as new again in the same snapshot.
        assert!(!projection.register(in_prev, 2));
        assert!(!projection.register(in_current, 2));
    }

    /// A forget that arrives while `acquire` waits for the next snapshot
    /// applies to THAT snapshot, not one later. The real per-snapshot order
    /// is `sync_forgets -> acquire -> sync_forgets -> admit*`.
    #[tokio::test]
    async fn a_forget_during_acquire_is_applied_to_that_snapshot() {
        let mut dedup = SnapshotDedup::new(FILTER_CAP);
        let forget = dedup.forget_handle();
        let t = raw_tx(5);
        let id = tx_id_of_raw(&t);
        assert_eq!(snapshot(&mut dedup, 1, &[&t]), vec![0], "first sight yields");

        // The forget happens during the wait for the snapshot.
        let acquired = dedup
            .around_acquire(async {
                forget.forget(id);
                Ok::<(), ()>(())
            })
            .await;
        assert!(acquired.is_ok());
        assert!(dedup.admit(&t, 1), "the acquired snapshot yields the forgotten tx");
        assert!(!dedup.admit(&t, 1), "and once only");
    }

    #[test]
    fn a_forget_of_an_unknown_id_changes_nothing() {
        let mut dedup = SnapshotDedup::new(FILTER_CAP);
        let forget = dedup.forget_handle();
        let t = raw_tx(3);
        assert_eq!(snapshot(&mut dedup, 1, &[&t]), vec![0]);
        forget.forget([0xee; 32]);
        assert_eq!(snapshot(&mut dedup, 1, &[&t]), Vec::<usize>::new());
    }

    /// The forget is a plain send. It returns while a draining snapshot
    /// holds the monitor mutex, and the next snapshot applies it.
    #[tokio::test]
    async fn the_forget_returns_while_a_snapshot_holds_the_monitor_lock() {
        let dedup = SnapshotDedup::new(FILTER_CAP);
        let forget = dedup.forget_handle();
        let state = Arc::new(Mutex::new(MonitorState { client: (), dedup }));
        let t = raw_tx(4);
        assert_eq!(snapshot(&mut state.lock().await.dedup, 1, &[&t]), vec![0]);

        let draining = state.lock().await;
        let (done_tx, done_rx) = std::sync::mpsc::channel();
        let id = tx_id_of_raw(&t);
        let caller = std::thread::spawn(move || {
            forget.forget(id);
            done_tx.send(()).unwrap();
        });
        done_rx
            .recv_timeout(std::time::Duration::from_secs(5))
            .expect("forget returned while the monitor lock was held");
        caller.join().unwrap();
        drop(draining);

        assert_eq!(
            snapshot(&mut state.lock().await.dedup, 1, &[&t]),
            vec![0],
            "the forget applied"
        );
    }

    // ---------------------------------------------------------------------
    // The forget path speaks in the log
    // ---------------------------------------------------------------------

    struct Capture(std::sync::Mutex<Vec<(log::Level, String)>>);

    impl log::Log for Capture {
        fn enabled(&self, _: &log::Metadata) -> bool {
            true
        }
        fn log(&self, record: &log::Record) {
            self.0.lock().unwrap().push((record.level(), record.args().to_string()));
        }
        fn flush(&self) {}
    }

    static CAPTURE: Capture = Capture(std::sync::Mutex::new(Vec::new()));

    /// Install the capturing logger (once per process; tests share it, so
    /// every assertion filters on its own tx id).
    fn capture_log() {
        let _ = log::set_logger(&CAPTURE);
        log::set_max_level(log::LevelFilter::Info);
    }

    fn lines_about(id: &TxId) -> Vec<(log::Level, String)> {
        let needle = hex(id);
        CAPTURE
            .0
            .lock()
            .unwrap()
            .iter()
            .filter(|(_, line)| line.contains(&needle))
            .cloned()
            .collect()
    }

    #[tokio::test]
    async fn the_forget_path_logs_queued_applied_and_re_yielded() {
        capture_log();
        let mut dedup = SnapshotDedup::new(FILTER_CAP);
        let forget = dedup.forget_handle();
        let t = raw_tx(6);
        let id = tx_id_of_raw(&t);
        let h = hex(&id);
        assert_eq!(snapshot(&mut dedup, 1, &[&t]), vec![0]);
        assert!(lines_about(&id).is_empty(), "a plain admit is silent");

        forget.forget(id);
        assert_eq!(
            lines_about(&id),
            vec![(log::Level::Info, format!("[MEMPOOL_FORGET] queued tx={h}"))]
        );

        dedup.around_acquire(std::future::ready(())).await;
        assert_eq!(
            lines_about(&id).last().cloned(),
            Some((log::Level::Info, format!("[MEMPOOL_FORGET] applied tx={h}")))
        );

        assert!(dedup.admit(&t, 1));
        assert_eq!(
            lines_about(&id).last().cloned(),
            Some((log::Level::Info, format!("[MEMPOOL_FORGET] re-yielded tx={h}")))
        );
        assert_eq!(lines_about(&id).len(), 3);

        // A later re-list is suppressed and says nothing more.
        assert!(!dedup.admit(&t, 1));
        assert_eq!(lines_about(&id).len(), 3, "re-yielded is logged once");
    }

    #[test]
    fn a_forget_after_the_monitor_is_gone_is_warned_not_swallowed() {
        capture_log();
        let dedup = SnapshotDedup::new(FILTER_CAP);
        let forget = dedup.forget_handle();
        drop(dedup);
        let id = tx_id_of_raw(&raw_tx(8));
        forget.forget(id);
        assert_eq!(
            lines_about(&id),
            vec![(
                log::Level::Warn,
                format!("[MEMPOOL_FORGET] dropped tx={} — the monitor is gone", hex(&id))
            )]
        );
    }

    #[test]
    fn the_client_hands_out_a_forget_handle_without_the_monitor_lock() {
        // Type-level: `forget_handle` takes `&self` and the handle is
        // `Clone + Send + Sync`, so it can be taken before `stream_updates`
        // consumes the client and moved into a consumer's sink.
        fn assert_handle<T: Clone + Send + Sync + 'static>() {}
        assert_handle::<ForgetHandle>();
        let _: fn(&LocalTxMonitorClient<()>) -> ForgetHandle = LocalTxMonitorClient::<()>::forget_handle;
    }
}
