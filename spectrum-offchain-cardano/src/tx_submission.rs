use std::collections::HashSet;
use std::fmt::{Display, Formatter};

use async_stream::stream;
use cml_core::serialization::Serialize;
use futures::channel::{mpsc, oneshot};
use futures::{SinkExt, Stream, StreamExt};
use log::{info, trace};
use pallas_network::miniprotocols::localtxsubmission;
use pallas_network::miniprotocols::localtxsubmission::RejectReason;

use cardano_submit_api::client::{Error, LocalTxSubmissionClient};
use spectrum_cardano_lib::OutputRef;
use spectrum_offchain::network::Network;

use crate::node_error::transcribe_bad_inputs_error;

pub struct TxSubmissionAgent<const ERA: u16, Tx> {
    client: LocalTxSubmissionClient<ERA, Tx>,
    mailbox: mpsc::Receiver<SubmitTx<Tx>>,
}

impl<const ERA: u16, Tx> TxSubmissionAgent<ERA, Tx> {
    pub fn new(
        client: LocalTxSubmissionClient<ERA, Tx>,
        buffer_size: usize,
    ) -> (Self, TxSubmissionChannel<ERA, Tx>) {
        let (snd, recv) = mpsc::channel(buffer_size);
        (
            Self {
                client,
                mailbox: recv,
            },
            TxSubmissionChannel(snd),
        )
    }
}

#[derive(Clone)]
pub struct TxSubmissionChannel<const ERA: u16, Tx>(mpsc::Sender<SubmitTx<Tx>>);

pub struct SubmitTx<Tx>(Tx, oneshot::Sender<SubmissionResult>);

#[derive(Debug, Clone)]
pub enum SubmissionResult {
    Ok,
    TxRejectedResult { rejected_bytes: Vec<u8> },
}

impl From<SubmissionResult> for Result<(), TxRejected> {
    fn from(value: SubmissionResult) -> Self {
        match value {
            SubmissionResult::Ok => Ok(()),
            SubmissionResult::TxRejectedResult {
                rejected_bytes: Some(error),
            } => {
                info!("error: {}", hex::encode(error.clone()));
                let missing_inputs = transcribe_bad_inputs_error(error);
                Err(if !missing_inputs.is_empty() {
                    TxRejected::MissingInputs(missing_inputs)
                } else {
                    TxRejected::Unknown
                })
            }
            _ => {
                info!("error with empty bytes");
                Err(TxRejected::Unknown)
            }
        }
    }
}

pub fn tx_submission_agent_stream<const ERA: u16, Tx>(
    mut agent: TxSubmissionAgent<ERA, Tx>,
) -> impl Stream<Item = ()>
where
    Tx: Serialize + Clone,
{
    stream! {
        loop {
            let SubmitTx(tx, on_resp) = agent.mailbox.select_next_some().await;
            loop {
                match agent.client.submit_tx(tx.clone()).await {
                    Ok(_) => on_resp.send(SubmissionResult::Ok).expect("Responder was dropped"),
                    Err(Error::TxSubmissionProtocol(err)) => {
                        trace!("Failed to submit TX: {}", hex::encode(tx.to_cbor_bytes()));
                        match err {
                            localtxsubmission::Error::TxRejected(RejectReason(rejected_bytes)) => {
                                trace!("TxRejected: Node responded with error: {}", hex::encode(&rejected_bytes));
                                on_resp.send(SubmissionResult::TxRejectedResult{rejected_bytes}).expect("Responder was dropped")
                            },
                            retryable_err => {
                                trace!("TxSubmissionProtocol responded with error: {}", retryable_err);
                                continue
                            },
                        };
                    },
                    Err(err) => panic!("Cannot submit TX due to {}", err),
                }
                break;
            }
        }
    }
}

#[derive(Debug)]
pub enum TxRejected {
    MissingInputs(HashSet<OutputRef>),
    Unknown,
}

impl TryFrom<TxRejected> for HashSet<OutputRef> {
    type Error = &'static str;
    fn try_from(value: TxRejected) -> Result<Self, Self::Error> {
        match value {
            TxRejected::MissingInputs(inputs) => Ok(inputs),
            TxRejected::Unknown => Err("Unknown rejection"),
        }
    }
}

impl Display for TxRejected {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match &self {
            TxRejected::MissingInputs(inputs) => write!(f, "TxRejected::MissingInputs({:?})", inputs),
            TxRejected::Unknown => write!(f, "TxRejected::Unknown"),
        }
    }
}

#[async_trait::async_trait]
impl<const ERA: u16, Tx> Network<Tx, TxRejected> for TxSubmissionChannel<ERA, Tx>
where
    Tx: Send,
{
    async fn submit_tx(&mut self, tx: Tx) -> Result<(), TxRejected> {
        let (snd, recv) = oneshot::channel();
        self.0.send(SubmitTx(tx, snd)).await.unwrap();
        recv.await.expect("Channel closed").into()
    }
}
