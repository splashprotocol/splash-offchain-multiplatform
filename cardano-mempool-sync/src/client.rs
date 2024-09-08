use std::collections::{HashSet, VecDeque};
use std::marker::PhantomData;
use std::path::Path;
use std::sync::Arc;

use async_stream::stream;
use cml_core::serialization::Deserialize;
use cml_crypto::blake2b224;
use futures::Stream;
use pallas_network::miniprotocols::{handshake, txmonitor, PROTOCOL_N2C_HANDSHAKE};
use pallas_network::multiplexer;
use pallas_network::multiplexer::{Bearer, RunningPlexer};
use tokio::sync::Mutex;

use crate::data::MempoolUpdate;

pub struct LocalTxMonitorClient<Tx> {
    plexer: RunningPlexer,
    tx_monitor: Arc<Mutex<MonitorState>>,
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

        let state = MonitorState {
            client: txmonitor::Client::new(tm_channel),
            filter: TxFilter::new(FILTER_CAP),
        };

        Ok(Self {
            plexer,
            tx_monitor: Arc::new(Mutex::new(state)),
            tx: PhantomData::default(),
        })
    }

    pub fn stream_updates<'a>(&'a self) -> impl Stream<Item = MempoolUpdate<Tx>> + Send + 'a
    where
        Tx: Deserialize,
    {
        stream! {
            loop {
                let mut tx_monitor = self.tx_monitor.lock().await;
                if let Ok(_) = tx_monitor.client.acquire().await {
                    loop {
                        if let Ok(Some(raw_tx)) = tx_monitor.client.query_next_tx().await {
                            let bytes = &*raw_tx.1;
                            if !tx_monitor.filter.register(hash_tx_bytes(bytes)) {
                                if let Some(tx) = Tx::from_cbor_bytes(bytes).ok() {
                                    yield MempoolUpdate::TxAccepted(tx);
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

#[derive(Copy, Clone, Eq, PartialEq, Hash)]
struct RawTxHash([u8; 28]);

struct TxFilter {
    known_txs: HashSet<RawTxHash>,
    eviction_queue: VecDeque<RawTxHash>,
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
    fn register(&mut self, tx: RawTxHash) -> bool {
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
}

fn hash_tx_bytes(tx: &[u8]) -> RawTxHash {
    RawTxHash(blake2b224(tx))
}

struct MonitorState {
    client: txmonitor::Client,
    filter: TxFilter,
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
