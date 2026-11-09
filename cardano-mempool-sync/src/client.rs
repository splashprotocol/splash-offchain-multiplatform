use std::collections::{HashSet, VecDeque};
use std::marker::PhantomData;
use std::path::Path;
use std::sync::Arc;

use async_stream::stream;
use cml_core::serialization::Deserialize;
use cml_crypto::blake2b256;
use futures::Stream;
use pallas_codec::minicbor::Decoder;
use pallas_network::miniprotocols::{handshake, txmonitor, PROTOCOL_N2C_HANDSHAKE};
use pallas_network::multiplexer;
use pallas_network::multiplexer::{Bearer, RunningPlexer};
use tokio::sync::{mpsc, Mutex};

pub struct LocalTxMonitorClient<Tx> {
    plexer: RunningPlexer,
    tx_monitor: Arc<Mutex<MonitorState>>,
    /// `the-forget-never-blocks-the-ledger-gate`: the sender side of the
    /// forget channel, kept outside the monitor mutex.
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

    /// `a-rollback-forgets-its-orphans-in-the-mempool-filter`: a handle that
    /// makes the monitor forget a tx id, so the next snapshot that lists the
    /// tx yields it again. Take it before [`Self::stream_updates`] consumes
    /// the client. It does not take the monitor mutex.
    pub fn forget_handle(&self) -> ForgetHandle {
        self.forget.clone()
    }

    pub fn stream_updates<'a>(self) -> impl Stream<Item = Tx> + Send + 'a
    where
        Tx: Deserialize + 'a,
    {
        stream! {
            loop {
                let mut guard = self.tx_monitor.lock().await;
                let MonitorState { client, dedup } = &mut *guard;
                // Forgets sent since the last snapshot apply before the next
                // one, so a re-admitted orphan is yielded on its first re-list.
                dedup.sync_forgets();
                if let Ok(_) = client.acquire().await {
                    loop {
                        if let Ok(Some(raw_tx)) = client.query_next_tx().await {
                            let bytes = &*raw_tx.1;
                            if dedup.admit(bytes) {
                                if let Some(tx) = Tx::from_cbor_bytes(bytes).ok() {
                                    yield tx;
                                }
                            }
                        } else {
                            break;
                        }
                    }
                }
            }
        }
    }

    pub async fn close(self) {
        self.plexer.abort().await
    }
}

const PROTOCOL_N2C_TX_MONITOR: u16 = 9;
const FILTER_CAP: usize = 4096;

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

fn body_span(bytes: &[u8]) -> Option<&[u8]> {
    let mut decoder = Decoder::new(bytes);
    // A definite or an indefinite array; its first element is the body.
    decoder.array().ok()?;
    let start = decoder.position();
    decoder.skip().ok()?;
    let end = decoder.position();
    bytes.get(start..end)
}

/// `the-forget-never-blocks-the-ledger-gate`: a cloneable handle that tells
/// the monitor to forget a tx id. `forget` only sends on an unbounded
/// channel: it never awaits and never takes the monitor mutex. The monitor
/// applies the forgets before its next snapshot.
#[derive(Clone)]
pub struct ForgetHandle(mpsc::UnboundedSender<TxId>);

impl ForgetHandle {
    /// Send `id` to the monitor and return. A closed monitor drops it.
    pub fn forget(&self, id: TxId) {
        let _ = self.0.send(id);
    }
}

/// The per-snapshot dedup of the mempool monitor: the filter of tx ids
/// already yielded and the receiver of the forgets.
///
/// `stream_updates` calls [`Self::sync_forgets`] before each `acquire` and
/// [`Self::admit`] for each tx the snapshot lists.
pub struct SnapshotDedup {
    filter: TxFilter,
    forget_rx: mpsc::UnboundedReceiver<TxId>,
    forget_tx: mpsc::UnboundedSender<TxId>,
}

impl SnapshotDedup {
    pub fn new(cap: usize) -> Self {
        let (forget_tx, forget_rx) = mpsc::unbounded_channel();
        Self {
            filter: TxFilter::new(cap),
            forget_rx,
            forget_tx,
        }
    }

    /// A handle that sends forgets to this dedup.
    pub fn forget_handle(&self) -> ForgetHandle {
        ForgetHandle(self.forget_tx.clone())
    }

    /// Apply every forget sent so far. Returns how many were applied.
    pub fn sync_forgets(&mut self) -> usize {
        let mut applied = 0;
        while let Ok(id) = self.forget_rx.try_recv() {
            self.filter.forget(&id);
            applied += 1;
        }
        applied
    }

    /// Register the raw tx. `true` when it is not yet known, so the snapshot
    /// yields it; `false` for a re-list.
    pub fn admit(&mut self, raw_tx: &[u8]) -> bool {
        !self.filter.register(tx_id_of_raw(raw_tx))
    }
}

struct TxFilter {
    known_txs: HashSet<TxId>,
    eviction_queue: VecDeque<TxId>,
    cap: usize,
}

impl TxFilter {
    fn new(cap: usize) -> Self {
        Self {
            known_txs: HashSet::with_capacity(cap),
            eviction_queue: VecDeque::with_capacity(cap),
            cap,
        }
    }
    fn register(&mut self, tx: TxId) -> bool {
        if self.known_txs.contains(&tx) {
            return true;
        }
        if self.known_txs.len() > self.cap {
            if let Some(candidate) = self.eviction_queue.pop_back() {
                self.known_txs.remove(&candidate);
            }
        }
        self.known_txs.insert(tx);
        self.eviction_queue.push_front(tx);
        false
    }

    /// Remove `tx` from the filter and the eviction queue.
    fn forget(&mut self, tx: &TxId) {
        if self.known_txs.remove(tx) {
            self.eviction_queue.retain(|known| known != tx);
        }
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

    fn hash_n(n: u16) -> TxId {
        let mut b = [0u8; 32];
        b[0] = (n >> 8) as u8;
        b[1] = n as u8;
        b
    }

    /// The feed contract downstream pool caches are built on (incident
    /// 2026-07-21: post-block snapshot re-lists rolled a last-writer-wins
    /// pool cache back behind its own in-flight tail): a tx delivered once
    /// is NEVER re-emitted, no matter how many acquire rounds re-list it.
    #[test]
    fn replayed_tx_is_never_re_emitted() {
        let mut filter = TxFilter::new(FILTER_CAP);
        let pool_chain: Vec<TxId> = (0..4).map(hash_n).collect();
        for h in &pool_chain {
            assert!(!filter.register(*h), "first observation is emitted");
        }
        // Arbitrarily many later snapshot re-lists (block re-observation,
        // acquire-loop restarts) — every replay is suppressed.
        for _round in 0..100 {
            for h in &pool_chain {
                assert!(filter.register(*h), "a replay must be deduped");
            }
        }
    }

    /// A seen tx is forgotten only after more than `cap` NEW txs displace it
    /// — at FILTER_CAP=4096 a seconds-scale replay window cannot evict.
    #[test]
    fn eviction_requires_cap_worth_of_new_txs() {
        let cap = 4;
        let mut filter = TxFilter::new(cap);
        let old = hash_n(u16::MAX);
        assert!(!filter.register(old));
        for i in 0..cap as u16 {
            assert!(!filter.register(hash_n(i)));
        }
        assert!(filter.register(old), "still deduped while within cap");
    }

    // ---------------------------------------------------------------------
    // `a-rollback-forgets-its-orphans-in-the-mempool-filter`
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

    /// One scripted snapshot, as `stream_updates` runs it: the forgets sent
    /// so far are applied before `acquire`, then every listed tx is
    /// registered. Returns the positions of the txs the snapshot yields.
    fn snapshot(dedup: &mut SnapshotDedup, txs: &[&[u8]]) -> Vec<usize> {
        dedup.sync_forgets();
        txs.iter()
            .enumerate()
            .filter(|(_, raw)| dedup.admit(raw))
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

    #[test]
    fn a_forgotten_tx_is_yielded_again_on_the_next_snapshot() {
        let mut dedup = SnapshotDedup::new(FILTER_CAP);
        let forget = dedup.forget_handle();
        let t = raw_tx(1);
        let other = raw_tx(2);
        assert_eq!(snapshot(&mut dedup, &[&t, &other]), vec![0, 1], "first sight yields both");
        assert_eq!(snapshot(&mut dedup, &[&t, &other]), Vec::<usize>::new(), "a re-list is suppressed");

        forget.forget(tx_id_of_raw(&t));
        assert_eq!(snapshot(&mut dedup, &[&t, &other]), vec![0], "only the forgotten tx comes back");
        assert_eq!(snapshot(&mut dedup, &[&t, &other]), Vec::<usize>::new(), "and once only");
    }

    #[test]
    fn a_forget_of_an_unknown_id_changes_nothing() {
        let mut dedup = SnapshotDedup::new(FILTER_CAP);
        let forget = dedup.forget_handle();
        let t = raw_tx(3);
        assert_eq!(snapshot(&mut dedup, &[&t]), vec![0]);
        forget.forget([0xee; 32]);
        assert_eq!(snapshot(&mut dedup, &[&t]), Vec::<usize>::new());
    }

    #[test]
    fn a_forget_takes_the_id_out_of_the_eviction_queue() {
        let cap = 2;
        let mut filter = TxFilter::new(cap);
        let t = hash_n(1);
        assert!(!filter.register(t));
        filter.forget(&t);
        assert!(!filter.known_txs.contains(&t));
        assert!(!filter.eviction_queue.contains(&t));
        // The FIFO still evicts only after more than `cap` new ids.
        assert!(!filter.register(t));
        for i in 10..10 + cap as u16 {
            assert!(!filter.register(hash_n(i)));
        }
        assert!(filter.register(t), "still deduped while within cap");
    }

    /// `the-forget-never-blocks-the-ledger-gate`: the forget is a plain
    /// send. It returns while a draining snapshot holds the monitor mutex,
    /// and the next snapshot applies it.
    #[tokio::test]
    async fn the_forget_returns_while_a_snapshot_holds_the_monitor_lock() {
        let dedup = SnapshotDedup::new(FILTER_CAP);
        let forget = dedup.forget_handle();
        let state = Arc::new(Mutex::new(MonitorState { client: (), dedup }));
        let t = raw_tx(4);
        assert_eq!(snapshot(&mut state.lock().await.dedup, &[&t]), vec![0]);

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

        assert_eq!(snapshot(&mut state.lock().await.dedup, &[&t]), vec![0], "the forget applied");
    }

    #[test]
    fn the_client_hands_out_a_forget_handle_without_the_monitor_lock() {
        // Type-level: `forget_handle` takes `&self` and the handle is
        // `Clone + Send + Sync`, so it can be taken before `stream_updates`
        // consumes the client and moved into the wallet's sink.
        fn assert_handle<T: Clone + Send + Sync + 'static>() {}
        assert_handle::<ForgetHandle>();
        let _: fn(&LocalTxMonitorClient<()>) -> ForgetHandle = LocalTxMonitorClient::<()>::forget_handle;
    }
}
