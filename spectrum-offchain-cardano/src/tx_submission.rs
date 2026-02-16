use crate::node::NodeConfig;
use crate::tx_tracker::TxTracker;
use async_stream::stream;
use cardano_submit_api::client::{Error, LocalTxSubmissionClient};
use cml_core::serialization::Serialize;
use futures::channel::{mpsc, oneshot};
use futures::{SinkExt, Stream, StreamExt};
use log::trace;
use pallas_network::miniprotocols::localstate::queries_v16::TransactionInput;
use pallas_network::miniprotocols::localtxsubmission::{
    ApplyTxError, ConwayLedgerFailure, ConwayUtxoWPredFailure, Response, TxValidationError, UtxoFailure,
};
use spectrum_cardano_lib::OutputRef;
use spectrum_offchain::network::Network;
use spectrum_offchain::tx_hash::CanonicalHash;
use std::collections::HashSet;
use std::fmt::{Display, Formatter};
use std::time::Duration;
use tokio::time::timeout;

pub struct TxSubmissionAgent<const ERA: u16, Tx, Tracker> {
    client: LocalTxSubmissionClient<ERA, Tx>,
    mailbox: mpsc::Receiver<SubmitTx<Tx>>,
    tracker: Tracker,
    node_config: NodeConfig,
}

impl<const ERA: u16, Tx, Tracker> TxSubmissionAgent<ERA, Tx, Tracker> {
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

pub struct TxSubmissionChannel<const ERA: u16, Tx>(mpsc::Sender<SubmitTx<Tx>>);

impl<const ERA: u16, Tx> Clone for TxSubmissionChannel<ERA, Tx> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

pub struct SubmitTx<Tx>(Tx, oneshot::Sender<SubmissionResult>);

#[derive(Debug, Clone)]
pub enum SubmissionResult {
    Ok,
    TxRejected { errors: TxRejection },
}

impl From<SubmissionResult> for Result<(), TxRejection> {
    fn from(value: SubmissionResult) -> Self {
        match value {
            SubmissionResult::Ok => Ok(()),
            SubmissionResult::TxRejected { errors } => Err(errors.into()),
        }
    }
}

const SUBMIT_TIMEOUT: Duration = Duration::from_secs(5);

pub fn tx_submission_agent_stream<'a, const ERA: u16, Tx, Tracker>(
    mut agent: TxSubmissionAgent<ERA, Tx, Tracker>,
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
            let submit_result = match timeout(SUBMIT_TIMEOUT, agent.client.submit_tx(tx.clone())).await {
                Ok(result) => result,
                Err(_) => panic!("Failed to submit TX {}: timeout", tx_hash),
            };
            match submit_result {
                Ok(Response::Accepted) => {
                    on_resp.send(SubmissionResult::Ok).expect("Responder was dropped");
                    agent.tracker.track(tx_hash, tx).await;
                },
                Ok(Response::Rejected(errors)) => {
                    trace!("TX {} was rejected due to error: {:?}", tx_hash, errors);
                    on_resp.send(SubmissionResult::TxRejected{errors: TxRejection(errors)}).expect("Responder was dropped");
                },
                Err(Error::TxSubmissionProtocol(err)) => {
                    panic!("Failed to submit TX {}: protocol returned error: {}", tx_hash, err);
                },
                Err(err) => panic!("Cannot submit TX {} due to {}", tx_hash, err),
            }
        }
    }
}

impl TryFrom<TxRejection> for HashSet<OutputRef> {
    type Error = &'static str;
    fn try_from(value: TxRejection) -> Result<Self, Self::Error> {
        let mut missing_utxos = HashSet::new();

        if let TxRejection(TxValidationError::ShelleyTxValidationError {
            error: ApplyTxError(node_errors),
            ..
        }) = value
        {
            for error in node_errors {
                if let ConwayLedgerFailure::UtxowFailure(ConwayUtxoWPredFailure::UtxoFailure(
                    UtxoFailure::BadInputsUTxO(inputs),
                )) = error
                {
                    missing_utxos.extend(inputs.into_iter().map(
                        |TransactionInput {
                             transaction_id,
                             index,
                         }| {
                            let tx_hash = *transaction_id;
                            OutputRef::new((*tx_hash).into(), *index)
                        },
                    ));
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
impl<const ERA: u16, Tx> Network<Tx, TxRejection> for TxSubmissionChannel<ERA, Tx>
where
    Tx: Send,
{
    async fn submit_tx(&mut self, tx: Tx) -> Result<(), TxRejection> {
        let (snd, recv) = oneshot::channel();
        self.0.send(SubmitTx(tx, snd)).await.unwrap();
        recv.await.expect("Channel closed").into()
    }
}

#[derive(Debug, Clone, derive_more::From)]
pub struct TxRejection(pub TxValidationError);

impl Display for TxRejection {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(format!("{:?}", self).as_str())
    }
}
