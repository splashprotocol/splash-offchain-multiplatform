use std::marker::PhantomData;
use std::path::Path;
use std::sync::{Arc, Mutex};

use async_stream::stream;
use cml_core::serialization::Deserialize;
use futures::Stream;
use pallas_network::miniprotocols::{handshake, PROTOCOL_N2C_HANDSHAKE, txmonitor};
use pallas_network::multiplexer;
use pallas_network::multiplexer::Bearer;
use tokio::task::JoinHandle;

use crate::data::MempoolUpdate;

pub struct LocalTxMonitorClient<Tx> {
    mplex_handle: JoinHandle<Result<(), multiplexer::Error>>,
    tx_monitor: Arc<Mutex<txmonitor::Client>>,
    tx: PhantomData<Tx>,
}

impl<Tx> LocalTxMonitorClient<Tx> {
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
            tx_monitor: Arc::new(Mutex::new(txmonitor::Client::new(tm_channel))),
            tx: PhantomData::default(),
        })
    }

    pub fn stream_updates<'a>(&'a self) -> impl Stream<Item = MempoolUpdate<Tx>> + 'a
    where
        Tx: Deserialize,
    {
        stream! {
            loop {
                if let Ok(_) = { self.tx_monitor.lock().unwrap() }.acquire().await {
                    loop {
                        if let Ok(Some(raw_tx)) = { self.tx_monitor.lock().unwrap() }.query_next_tx().await {
                            if let Ok(tx) = Tx::from_cbor_bytes(&*raw_tx.1)
                                .map(MempoolUpdate::TxAccepted) {
                                yield tx;
                            }
                        } else {
                            break;
                        }
                    }
                }
            }
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
