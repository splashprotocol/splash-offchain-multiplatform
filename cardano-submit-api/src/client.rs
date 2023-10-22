use std::path::Path;

use cml_chain::transaction::Transaction;
use cml_core::serialization::Serialize;
use pallas_network::miniprotocols::handshake::RefuseReason;
use pallas_network::miniprotocols::localtxsubmission::{EraTx, RejectReason};
use pallas_network::miniprotocols::{
    handshake, localtxsubmission, PROTOCOL_N2C_HANDSHAKE, PROTOCOL_N2C_TX_SUBMISSION,
};
use pallas_network::multiplexer;
use pallas_network::multiplexer::Bearer;
use tokio::task::JoinHandle;

pub struct LocalTxSubmissionClient<const ERA: u16> {
    mplex_handle: JoinHandle<Result<(), multiplexer::Error>>,
    tx_submission: localtxsubmission::Client,
}

impl<const EraId: u16> LocalTxSubmissionClient<EraId> {
    #[cfg(not(target_os = "windows"))]
    pub async fn init<'a>(path: impl AsRef<Path>, magic: u64) -> Result<Self, Error> {
        let bearer = Bearer::connect_unix(path).await.map_err(Error::ConnectFailure)?;

        let mut mplex = multiplexer::Plexer::new(bearer);

        let hs_channel = mplex.subscribe_client(PROTOCOL_N2C_HANDSHAKE);
        let ts_channel = mplex.subscribe_client(PROTOCOL_N2C_TX_SUBMISSION);

        let mplex_handle = tokio::spawn(async move { mplex.run().await });

        let versions = handshake::n2c::VersionTable::v10_and_above(magic);
        let mut client = handshake::Client::new(hs_channel);

        let handshake = client
            .handshake(versions)
            .await
            .map_err(Error::HandshakeProtocol)?;

        if let handshake::Confirmation::Rejected(reason) = handshake {
            return Err(Error::HandshakeRefused(reason));
        }

        let ts_client = localtxsubmission::Client::new(ts_channel);

        Ok(Self {
            mplex_handle,
            tx_submission: ts_client,
        })
    }

    pub async fn submit_tx(&mut self, tx: Transaction) -> Result<(), Error> {
        let tx_bytes = tx.to_cbor_bytes();
        self.tx_submission
            .submit_tx(EraTx(EraId, tx_bytes))
            .await
            .map_err(Error::TxSubmissionProtocol)?;
        Ok(())
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

    #[error("chain-sync protocol error")]
    TxSubmissionProtocol(#[source] localtxsubmission::Error<RejectReason>),

    #[error("handshake version not accepted")]
    HandshakeRefused(RefuseReason),
}
