use std::collections::HashSet;
use std::fmt::Display;

use async_stream::stream;
use cml_core::serialization::Serialize;
use futures::channel::{mpsc, oneshot};
use futures::{SinkExt, Stream, StreamExt};
use log::{trace, warn};
use pallas_network::miniprotocols::localtxsubmission;
use pallas_network::miniprotocols::localtxsubmission::cardano_node_errors::{
    ApplyTxError, ConwayLedgerPredFailure, ConwayUtxoPredFailure, ConwayUtxowPredFailure, TxInput,
};
use pallas_network::miniprotocols::localtxsubmission::Response;
use pallas_network::multiplexer;

use crate::node::NodeConfig;
use crate::tx_tracker::TxTracker;
use cardano_submit_api::client::{Error, LocalTxSubmissionClient};
use spectrum_cardano_lib::OutputRef;
use spectrum_offchain::network::Network;
use spectrum_offchain::tx_hash::CanonicalHash;

pub struct TxSubmissionAgent<'a, const ERA: u16, Tx, Tracker> {
    client: LocalTxSubmissionClient<'a, ERA, Tx>,
    mailbox: mpsc::Receiver<SubmitTx<Tx>>,
    tracker: Tracker,
    node_config: NodeConfig,
}

impl<'a, const ERA: u16, Tx, Tracker> TxSubmissionAgent<'a, ERA, Tx, Tracker> {
    pub async fn new(
        tracker: Tracker,
        node_config: NodeConfig,
        buffer_size: usize,
    ) -> Result<(Self, TxSubmissionChannel<ERA, Tx>), Error> {
        let tx_submission_client =
            LocalTxSubmissionClient::init(node_config.path.clone(), node_config.magic).await?;
        let (snd, recv) = mpsc::channel(buffer_size);
        let agent = Self {
            client: tx_submission_client,
            mailbox: recv,
            tracker,
            node_config,
        };
        Ok((agent, TxSubmissionChannel(snd)))
    }

    pub fn recover(&mut self) {
        self.client.unsafe_reset();
    }

    pub async fn restarted(self) -> Result<Self, Error> {
        trace!("Restarting TxSubmissionProtocol");
        let TxSubmissionAgent {
            client,
            mailbox,
            tracker,
            node_config,
        } = self;
        client.close().await;
        let new_tx_submission_client =
            LocalTxSubmissionClient::init(node_config.path.clone(), node_config.magic).await?;
        Ok(Self {
            client: new_tx_submission_client,
            mailbox,
            tracker,
            node_config,
        })
    }
}

#[derive(Clone)]
pub struct TxSubmissionChannel<const ERA: u16, Tx>(mpsc::Sender<SubmitTx<Tx>>);

pub struct SubmitTx<Tx>(Tx, oneshot::Sender<SubmissionResult>);

#[derive(Debug, Clone)]
pub enum SubmissionResult {
    Ok,
    TxRejected { errors: RejectReasons },
}

impl From<SubmissionResult> for Result<(), RejectReasons> {
    fn from(value: SubmissionResult) -> Self {
        match value {
            SubmissionResult::Ok => Ok(()),
            SubmissionResult::TxRejected { errors } => Err(errors.into()),
        }
    }
}

pub fn tx_submission_agent_stream<'a, const ERA: u16, Tx, Tracker>(
    mut agent: TxSubmissionAgent<'a, ERA, Tx, Tracker>,
) -> impl Stream<Item = ()> + 'a
where
    Tx: CanonicalHash + Serialize + Clone + 'a,
    Tx::Hash: Display,
    Tracker: TxTracker<Tx::Hash, Tx> + 'a,
{
    stream! {
        loop {
            let SubmitTx(tx, on_resp) = agent.mailbox.select_next_some().await;
            let tx_hash = tx.canonical_hash();
            let tx: Tx = tx.into();
            match agent.client.submit_tx(tx.clone()).await {
                Ok(Response::Accepted) => {
                    on_resp.send(SubmissionResult::Ok).expect("Responder was dropped");
                    agent.tracker.track(tx_hash, tx).await;
                },
                Ok(Response::Rejected(errors)) => {
                    trace!("TX {} was rejected due to error: {:?}", tx_hash, errors);
                    on_resp.send(SubmissionResult::TxRejected{errors:  RejectReasons(Some(errors))}).expect("Responder was dropped");
                },
                Err(Error::TxSubmissionProtocol(err)) => {
                    match err {
                        localtxsubmission::Error::ChannelError(multiplexer::Error::Decoding(_)) => {
                            warn!("TX {} was likely rejected, reason unknown. Trying to recover.", tx_hash);
                            agent.recover();
                            on_resp.send(SubmissionResult::TxRejected{errors: RejectReasons(None)}).expect("Responder was dropped");
                        }
                        retryable_err => {
                            trace!("Failed to submit TX {}: protocol returned error: {}", tx_hash, retryable_err);
                            agent = agent.restarted().await.expect("Failed to restart TxSubmissionProtocol");
                            on_resp.send(SubmissionResult::TxRejected{errors: RejectReasons(None)}).expect("Responder was dropped");
                        },
                    };
                },
                Err(err) => panic!("Cannot submit TX {} due to {}", tx_hash, err),
            }
        }
    }
}

impl TryFrom<RejectReasons> for HashSet<OutputRef> {
    type Error = &'static str;
    fn try_from(value: RejectReasons) -> Result<Self, Self::Error> {
        let mut missing_utxos = HashSet::new();

        if let Some(ApplyTxError { node_errors }) = value.0 {
            for error in node_errors {
                if let ConwayLedgerPredFailure::UtxowFailure(ConwayUtxowPredFailure::UtxoFailure(
                    ConwayUtxoPredFailure::BadInputsUtxo(inputs),
                )) = error
                {
                    missing_utxos.extend(inputs.into_iter().map(|TxInput { tx_hash, index }| {
                        let tx_hash = *tx_hash;
                        OutputRef::new(tx_hash.into(), index)
                    }));
                }
            }
        }

        if !missing_utxos.is_empty() {
            Ok(missing_utxos)
        } else {
            Err("No missing inputs")
        }
    }
}

#[async_trait::async_trait]
impl<const ERA: u16, Tx> Network<Tx, RejectReasons> for TxSubmissionChannel<ERA, Tx>
where
    Tx: Send,
{
    async fn submit_tx(&mut self, tx: Tx) -> Result<(), RejectReasons> {
        let (snd, recv) = oneshot::channel();
        self.0.send(SubmitTx(tx, snd)).await.unwrap();
        recv.await.expect("Channel closed").into()
    }
}

#[derive(Debug, Clone, derive_more::Display, derive_more::From)]
#[display("RejectReasons: {:?}", "_0")]
pub struct RejectReasons(pub Option<ApplyTxError>);
