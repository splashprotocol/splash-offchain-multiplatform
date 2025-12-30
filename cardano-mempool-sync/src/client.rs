use std::collections::HashSet;
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
            mempool: MempoolProjection::new(FILTER_CAP),
        };

        Ok(Self {
            plexer,
            tx_monitor: Arc::new(Mutex::new(state)),
            tx: PhantomData::default(),
        })
    }

    pub fn stream_updates<'a>(self) -> impl Stream<Item = Tx> + Send + 'a
    where
        Tx: Deserialize + 'a,
    {
        stream! {
            let mut seq_num = 0;
            loop {
                let mut tx_monitor = self.tx_monitor.lock().await;
                if let Ok(_) = tx_monitor.client.acquire().await {
                    while let Ok(Some(raw_tx)) = tx_monitor.client.query_next_tx().await {
                        let bytes = &*raw_tx.1;
                        if !tx_monitor.mempool.register(hash_tx_bytes(bytes), seq_num) {
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

#[derive(Copy, Clone, Eq, PartialEq, Hash)]
struct RawTxHash([u8; 28]);

struct MempoolProjection {
    prev_projection: HashSet<RawTxHash>,
    current_projection: HashSet<RawTxHash>,
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
    fn register(&mut self, tx: RawTxHash, slot: u64) -> bool {
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
}

fn hash_tx_bytes(tx: &[u8]) -> RawTxHash {
    RawTxHash(blake2b224(tx))
}

struct MonitorState {
    client: txmonitor::Client,
    mempool: MempoolProjection,
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

    fn make_tx_hash(byte: u8) -> RawTxHash {
        RawTxHash([byte; 28])
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
        assert_eq!(projection.register(tx1, 2), true, "tx1 at slot 2 should be duplicate");
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
        assert_eq!(projection.register(tx1, 4), false, "tx1 at slot 4 should be new (reappeared)");
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
}
