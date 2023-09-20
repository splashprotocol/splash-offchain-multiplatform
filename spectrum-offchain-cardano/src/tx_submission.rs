use async_stream::stream;
use cml_chain::transaction::Transaction;
use derive_more::Display;
use futures::channel::{mpsc, oneshot};
use futures::{SinkExt, Stream, StreamExt};

use cardano_submit_api::client::{Error, LocalTxSubmissionClient};
use spectrum_offchain::network::Network;

pub struct TxSubmissionAgent<const ERA: u16> {
    client: LocalTxSubmissionClient<ERA>,
    mailbox: mpsc::Receiver<SubmitTx>,
}

impl<const ERA: u16> TxSubmissionAgent<ERA> {
    pub fn new(client: LocalTxSubmissionClient<ERA>, buffer_size: usize) -> (Self, TxSubmissionChannel<ERA>) {
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
pub struct TxSubmissionChannel<const ERA: u16>(mpsc::Sender<SubmitTx>);

pub struct SubmitTx(Transaction, oneshot::Sender<SubmissionResult>);

#[derive(Debug, Copy, Clone)]
pub enum SubmissionResult {
    Ok,
    TxRejected,
}

impl From<SubmissionResult> for Result<(), TxRejected> {
    fn from(value: SubmissionResult) -> Self {
        match value {
            SubmissionResult::Ok => Ok(()),
            SubmissionResult::TxRejected => Err(TxRejected),
        }
    }
}

pub fn tx_submission_agent_stream<const EraId: u16>(
    mut agent: TxSubmissionAgent<EraId>,
) -> impl Stream<Item = ()> {
    stream! {
        loop {
            let SubmitTx(tx, on_resp) = agent.mailbox.select_next_some().await;
            match agent.client.submit_tx(tx).await {
                Ok(_) => on_resp.send(SubmissionResult::Ok).expect("Responder was dropped"),
                Err(Error::TxSubmissionProtocol(_)) => on_resp.send(SubmissionResult::TxRejected).expect("Responder was dropped"),
                Err(_) => panic!("Cannot submit"),
            }
        }
    }
}

#[derive(Debug, Display)]
pub struct TxRejected;

#[async_trait::async_trait]
impl<const ERA: u16> Network<Transaction, TxRejected> for TxSubmissionChannel<ERA> {
    async fn submit_tx(&mut self, tx: Transaction) -> Result<(), TxRejected> {
        let (snd, recv) = oneshot::channel();
        self.0.send(SubmitTx(tx, snd)).await.unwrap();
        recv.await.expect("Channel closed").into()
    }
}
