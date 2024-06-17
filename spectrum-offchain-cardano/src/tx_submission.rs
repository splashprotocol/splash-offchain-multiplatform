use std::collections::HashSet;
use std::fmt::{Display, Formatter};
use std::ops::Deref;

use async_stream::stream;
use cml_core::serialization::Serialize;
use futures::channel::{mpsc, oneshot};
use futures::{SinkExt, Stream, StreamExt};
use log::{trace, warn};
use pallas_network::miniprotocols::localtxsubmission;
use pallas_network::miniprotocols::localtxsubmission::RejectReason;
use pallas_network::multiplexer;

use cardano_submit_api::client::{Error, LocalTxSubmissionClient};
use spectrum_cardano_lib::OutputRef;
use spectrum_offchain::network::Network;
use spectrum_offchain::tx_hash;
use spectrum_offchain::tx_hash::CanonicalHash;

use crate::node::NodeConfig;
use crate::node_error::transcribe_bad_inputs_error;

pub struct TxSubmissionAgent<'a, const ERA: u16, TxAdapter, Tx> {
    client: LocalTxSubmissionClient<ERA, Tx>,
    mailbox: mpsc::Receiver<SubmitTx<TxAdapter>>,
    node_config: NodeConfig<'a>,
}

impl<'a, const ERA: u16, TxAdapter, Tx> TxSubmissionAgent<'a, ERA, TxAdapter, Tx> {
    pub async fn new(
        node_config: NodeConfig<'a>,
        buffer_size: usize,
    ) -> Result<(Self, TxSubmissionChannel<ERA, TxAdapter>), Error> {
        let tx_submission_client = LocalTxSubmissionClient::init(node_config.path, node_config.magic).await?;
        let (snd, recv) = mpsc::channel(buffer_size);
        let agent = Self {
            client: tx_submission_client,
            mailbox: recv,
            node_config,
        };
        Ok((agent, TxSubmissionChannel(snd)))
    }

    pub fn recover(&mut self) {
        self.client.unsafe_reset();
    }

    pub async fn restarted(self) -> Result<Self, Error> {
        let TxSubmissionAgent {
            client,
            mailbox,
            node_config,
        } = self;
        client.close().await;
        let new_tx_submission_client =
            LocalTxSubmissionClient::init(node_config.path, node_config.magic).await?;
        Ok(Self {
            client: new_tx_submission_client,
            mailbox,
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
    TxRejected { rejected_bytes: Vec<u8> },
}

impl From<SubmissionResult> for Result<(), TxRejected> {
    fn from(value: SubmissionResult) -> Self {
        match value {
            SubmissionResult::Ok => Ok(()),
            SubmissionResult::TxRejected { rejected_bytes } => {
                let missing_inputs = transcribe_bad_inputs_error(rejected_bytes);
                Err(if !missing_inputs.is_empty() {
                    TxRejected::MissingInputs(missing_inputs)
                } else {
                    TxRejected::Unknown
                })
            }
        }
    }
}

const MAX_SUBMIT_ATTEMPTS: usize = 3;

pub fn tx_submission_agent_stream<'a, const ERA: u16, TxAdapter, Tx>(
    mut agent: TxSubmissionAgent<'a, ERA, TxAdapter, Tx>,
) -> impl Stream<Item = ()> + 'a
where
    TxAdapter: Deref<Target = Tx> + CanonicalHash + 'a,
    TxAdapter::Hash: Display,
    Tx: Serialize + Clone + 'a,
{
    stream! {
        loop {
            let SubmitTx(tx, on_resp) = agent.mailbox.select_next_some().await;
            let mut attempts_done = 0;
            let tx_hash = tx.canonical_hash();
            loop {
                match agent.client.submit_tx((*tx).clone()).await {
                    Ok(_) => on_resp.send(SubmissionResult::Ok).expect("Responder was dropped"),
                    Err(Error::TxSubmissionProtocol(err)) => {
                        trace!("Failed to submit TX {}: {}", tx_hash, hex::encode(tx.to_cbor_bytes()));
                        match err {
                            localtxsubmission::Error::TxRejected(RejectReason(rejected_bytes)) => {
                                trace!("TX {} was rejected due to error: {}", tx_hash, hex::encode(&rejected_bytes));
                                on_resp.send(SubmissionResult::TxRejected{rejected_bytes}).expect("Responder was dropped");
                            },
                            localtxsubmission::Error::InboundChannelError(multiplexer::Error::Decoding(_)) => {
                                warn!("TX {} was likely rejected, reason unknown. Trying to recover.", tx_hash);
                                agent.recover();
                                on_resp.send(SubmissionResult::TxRejected{rejected_bytes: vec![]}).expect("Responder was dropped");
                            }
                            retryable_err => {
                                trace!("Failed to submit TX {}: protocol returned error: {}", tx_hash, retryable_err);
                                if attempts_done < MAX_SUBMIT_ATTEMPTS {
                                    trace!("Retrying");
                                    attempts_done += 1;
                                } else {
                                    trace!("Restarting TxSubmissionProtocol");
                                    agent = agent.restarted().await.expect("Failed to restart TxSubmissionProtocol");
                                }
                                continue;
                            },
                        };
                    },
                    Err(err) => panic!("Cannot submit TX {} due to {}", tx_hash, err),
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
