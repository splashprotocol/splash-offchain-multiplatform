use std::path::Path;

use cml_chain::transaction::Transaction;
use cml_core::serialization::Deserialize;
use pallas_network::miniprotocols::{handshake, txmonitor, PROTOCOL_N2C_HANDSHAKE};
use pallas_network::multiplexer;
use pallas_network::multiplexer::Bearer;
use tokio::task::JoinHandle;

pub struct LocalTxMonitorClient {
    mplex_handle: JoinHandle<Result<(), multiplexer::Error>>,
    tx_monitor: txmonitor::Client,
}

impl LocalTxMonitorClient {
    #[cfg(not(target_os = "windows"))]
    pub async fn connect(path: impl AsRef<Path>, magic: u64) -> Result<Self, Error> {
        let bearer = Bearer::connect_unix(path).await.map_err(Error::ConnectFailure)?;

        let mut mplex = multiplexer::Plexer::new(bearer);

        let hs_channel = mplex.subscribe_client(PROTOCOL_N2C_HANDSHAKE);
        let tm_channel = mplex.subscribe_client(PROTOCOL_N2C_TX_MONITOR);

        let mplex_handle = tokio::spawn(async move { mplex.run().await });

        let versions = handshake::n2c::VersionTable::v10_and_above(magic);
        let mut client = handshake::Client::new(hs_channel);

        let handshake = client
            .handshake(versions)
            .await
            .map_err(Error::HandshakeProtocol)?;

        if let handshake::Confirmation::Rejected(_reason) = handshake {
            return Err(Error::IncompatibleVersion);
        }

        Ok(Self {
            mplex_handle,
            tx_monitor: txmonitor::Client::new(tm_channel),
        })
    }

    pub async fn try_pull_next(&mut self) -> Option<Transaction> {
        if let Ok(maybe_tx) = self.tx_monitor.query_next_tx().await {
            maybe_tx.and_then(|raw_tx| Transaction::from_cbor_bytes(&*raw_tx.1).ok())
        } else {
            None
        }
    }

    pub fn close(self) {
        self.mplex_handle.abort()
    }
}

const PROTOCOL_N2C_TX_MONITOR: u16 = 9;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("error connecting bearer")]
    ConnectFailure(#[source] tokio::io::Error),

    #[error("handshake protocol error")]
    HandshakeProtocol(handshake::Error),

    #[error("handshake version not accepted")]
    IncompatibleVersion,
}
