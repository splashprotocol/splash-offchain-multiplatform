use async_stream::stream;
use cml_core::serialization::Serialize;
use futures::channel::{mpsc, oneshot};
use futures::{SinkExt, Stream, StreamExt};
use pallas_network::miniprotocols::localtxsubmission;
use std::fmt::{Display, Formatter};

use cardano_submit_api::client::{Error, LocalTxSubmissionClient};
use spectrum_offchain::network::Network;

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
    TxRejectedResult { rejected_bytes: Option<Vec<u8>> },
}

impl From<SubmissionResult> for Result<(), TxRejected> {
    fn from(value: SubmissionResult) -> Self {
        match value {
            SubmissionResult::Ok => Ok(()),
            SubmissionResult::TxRejectedResult { rejected_bytes } => Err(TxRejected {
                msg_bytes: rejected_bytes,
            }),
        }
    }
}

pub fn tx_submission_agent_stream<const EraId: u16, Tx>(
    mut agent: TxSubmissionAgent<EraId, Tx>,
) -> impl Stream<Item = ()>
where
    Tx: Serialize,
{
    stream! {
        loop {
            let SubmitTx(tx, on_resp) = agent.mailbox.select_next_some().await;
            match agent.client.submit_tx(tx).await {
                Ok(_) => on_resp.send(SubmissionResult::Ok).expect("Responder was dropped"),
                Err(Error::TxSubmissionProtocol(err)) => {
                    match err {
                        localtxsubmission::Error::TxRejected(bytes) => {
                            on_resp.send(SubmissionResult::TxRejectedResult{rejected_bytes: Some(bytes.0)}).expect("Responder was dropped")
                        },
                        _ => {
                            on_resp.send(SubmissionResult::TxRejectedResult{rejected_bytes: None}).expect("Responder was dropped")
                        }
                    };
                },
                Err(_) => panic!("Cannot submit"),
            }
        }
    }
}

#[derive(Debug)]
pub struct TxRejected {
    pub msg_bytes: Option<Vec<u8>>,
}

impl Display for TxRejected {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match &self.msg_bytes.clone() {
            None => write!(f, "TxRejected (None)"),
            Some(bytes) => write!(f, "TxRejected (Hex: {})", hex::encode(bytes)),
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
