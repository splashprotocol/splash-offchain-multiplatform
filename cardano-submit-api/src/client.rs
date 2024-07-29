use std::marker::PhantomData;
use std::path::Path;

use cml_core::serialization::Serialize;
use cml_crypto::blake2b256;
use log::{trace, warn};
use pallas_network::miniprotocols::handshake::RefuseReason;
use pallas_network::miniprotocols::localtxsubmission::cardano_node_errors::ApplyTxError;
use pallas_network::miniprotocols::localtxsubmission::{EraTx, NodeErrorDecoder, Response};
use pallas_network::miniprotocols::{
    handshake, localtxsubmission, PROTOCOL_N2C_HANDSHAKE, PROTOCOL_N2C_TX_SUBMISSION,
};
use pallas_network::multiplexer;
use pallas_network::multiplexer::{Bearer, RunningPlexer};

pub struct LocalTxSubmissionClient<'a, const ERA: u16, Tx> {
    plexer: RunningPlexer,
    tx_submission: localtxsubmission::Client<'a, NodeErrorDecoder>,
    tx: PhantomData<Tx>,
}

impl<'a, const ERA: u16, Tx> LocalTxSubmissionClient<'a, ERA, Tx> {
    #[cfg(not(target_os = "windows"))]
    pub async fn init(path: impl AsRef<Path>, magic: u64) -> Result<Self, Error> {
        let bearer = Bearer::connect_unix(path).await.map_err(Error::ConnectFailure)?;

        let mut mplex = multiplexer::Plexer::new(bearer);

        let hs_channel = mplex.subscribe_client(PROTOCOL_N2C_HANDSHAKE);
        let ts_channel = mplex.subscribe_client(PROTOCOL_N2C_TX_SUBMISSION);

        let plexer = mplex.spawn();

        let versions = handshake::n2c::VersionTable::v10_and_above(magic);
        let mut client = handshake::Client::new(hs_channel);

        let handshake = client
            .handshake(versions)
            .await
            .map_err(Error::HandshakeProtocol)?;

        if let handshake::Confirmation::Rejected(reason) = handshake {
            return Err(Error::HandshakeRefused(reason));
        }

        let ts_client = localtxsubmission::Client::new(ts_channel, NodeErrorDecoder::default());

        Ok(Self {
            plexer,
            tx_submission: ts_client,
            tx: PhantomData::default(),
        })
    }

    pub async fn submit_tx(&mut self, tx: Tx) -> Result<Response<Vec<ApplyTxError>>, Error>
    where
        Tx: Serialize,
    {
        let tx_bytes = tx.to_cbor_bytes();
        let hash = hex::encode(&blake2b256(&tx_bytes)[0..8]);
        trace!("[{}] Going to submit TX", hash);
        let result = self
            .tx_submission
            .submit_tx(EraTx(ERA, tx_bytes))
            .await
            .map_err(Error::TxSubmissionProtocol);
        trace!("[{}] Submit attempt finished", hash);
        result
    }

    pub async fn close(self) {
        self.plexer.abort().await
    }

    pub fn unsafe_reset(&mut self) {
        warn!("Unsafe client reset was requested, but it is no longer supported!");
    }
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("error connecting bearer")]
    ConnectFailure(#[source] tokio::io::Error),

    #[error("handshake protocol error")]
    HandshakeProtocol(handshake::Error),

    #[error("chain-sync protocol error")]
    TxSubmissionProtocol(#[source] localtxsubmission::Error),

    #[error("handshake version not accepted")]
    HandshakeRefused(RefuseReason),
}
