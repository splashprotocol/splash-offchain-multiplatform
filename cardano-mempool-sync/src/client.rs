use std::path::Path;

use pallas_network::miniprotocols::{handshake, PROTOCOL_N2C_HANDSHAKE, txmonitor};
use pallas_network::multiplexer;
use pallas_network::multiplexer::Bearer;
use pallas_primitives::babbage;
use tokio::task::JoinHandle;

const PROTOCOL_N2C_TX_MONITOR: u16 = 9;

pub struct LocalTxMonitorClient {
    mplex_handle: JoinHandle<Result<(), multiplexer::Error>>,
    tx_monitor: txmonitor::Client,
}

impl LocalTxMonitorClient {
    #[cfg(not(target_os = "windows"))]
    pub async fn connect(path: impl AsRef<Path>, magic: u64) -> Result<Self, Error> {
        let bearer = Bearer::connect_unix(path)
            .await
            .map_err(Error::ConnectFailure)?;

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

        if let handshake::Confirmation::Rejected(reason) = handshake {
            return Err(Error::IncompatibleVersion);
        }

        Ok(Self {
            mplex_handle,
            tx_monitor: txmonitor::Client::new(tm_channel),
        })
    }

    pub async fn try_pull_next(&mut self) -> Option<babbage::Tx> {
        if let Ok(maybe_tx) = self.tx_monitor.query_next_tx().await {
            maybe_tx.and_then(|raw_tx| minicbor::decode(&*raw_tx).ok())
        } else {
            None
        }
    }

    pub fn close(self) {
        self.mplex_handle.abort()
    }
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
