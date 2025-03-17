use async_stream::stream;
use cml_crypto::Serialize;
use futures::channel::mpsc;
use futures::{SinkExt, Stream, StreamExt};
use log::{info, trace, warn};
use pallas_network::miniprotocols::localtxsubmission;
use pallas_network::miniprotocols::localtxsubmission::Response;
use pallas_network::multiplexer;

use cardano_submit_api::client::{Error, LocalTxSubmissionClient};
use spectrum_offchain_cardano::node::NodeConfig;

pub struct TxSubmissionAgent<'a, const ERA: u16, Tx> {
    client: LocalTxSubmissionClient<'a, ERA, Tx>,
    mailbox: mpsc::Receiver<Tx>,
    node_config: NodeConfig,
}

impl<'a, const ERA: u16, Tx> TxSubmissionAgent<'a, ERA, Tx> {
    pub async fn new(
        node_config: NodeConfig,
        buffer_size: usize,
    ) -> Result<(Self, TxSubmissionChannel<ERA, Tx>), Error> {
        let tx_submission_client =
            LocalTxSubmissionClient::init(node_config.path.clone(), node_config.magic).await?;
        let (snd, recv) = mpsc::channel(buffer_size);
        let agent = Self {
            client: tx_submission_client,
            mailbox: recv,
            node_config,
        };
        Ok((agent, TxSubmissionChannel(snd)))
    }

    pub fn stream(mut self) -> impl Stream<Item = ()> + 'a
    where
        Tx: Serialize + 'a,
    {
        stream! {
            loop {
                let tx = self.mailbox.select_next_some().await;
                match self.client.submit_tx(tx).await {
                    Ok(Response::Accepted) => {
                        info!("Tx was accepted");
                    },
                    Ok(Response::Rejected(errors)) => {
                        trace!("Tx was rejected due to error: {:?}", errors);
                    },
                    Err(Error::TxSubmissionProtocol(err)) => {
                        match err {
                            localtxsubmission::Error::ChannelError(multiplexer::Error::Decoding(_)) => {
                                warn!("Tx was likely rejected, reason unknown. Trying to recover.");
                                self.recover();
                            }
                            retryable_err => {
                                trace!("Failed to submit Tx: protocol returned error: {}", retryable_err);
                                self = self.restarted().await.expect("Failed to restart TxSubmissionProtocol");
                            },
                        };
                    },
                    Err(err) => panic!("Cannot submit Tx due to {}", err),
                }
            }
        }
    }

    pub fn recover(&mut self) {
        self.client.unsafe_reset();
    }

    pub async fn restarted(self) -> Result<Self, Error> {
        trace!("Restarting TxSubmissionProtocol");
        let TxSubmissionAgent {
            client,
            mailbox,
            node_config,
        } = self;
        client.close().await;
        let new_tx_submission_client =
            LocalTxSubmissionClient::init(node_config.path.clone(), node_config.magic).await?;
        Ok(Self {
            client: new_tx_submission_client,
            mailbox,
            node_config,
        })
    }
}

pub struct TxSubmissionChannel<const ERA: u16, Tx>(mpsc::Sender<Tx>);

impl<const ERA: u16, Tx> Clone for TxSubmissionChannel<ERA, Tx> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<const ERA: u16, Tx> TxSubmissionChannel<ERA, Tx> {
    pub async fn submit(&mut self, tx: Tx) {
        self.0.send(tx).await.unwrap();
    }
}
