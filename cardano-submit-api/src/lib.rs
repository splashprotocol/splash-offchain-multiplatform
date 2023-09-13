use cml_chain::transaction::Transaction;

use crate::client::LocalTxSubmissionClient;

pub mod client;

pub struct SubmitTxFailure;

#[async_trait::async_trait]
pub trait SubmitTx {
    async fn submit(&mut self, tx: Transaction) -> Result<(), SubmitTxFailure>;
}

#[async_trait::async_trait]
impl SubmitTx for LocalTxSubmissionClient {
    async fn submit(&mut self, tx: Transaction) -> Result<(), SubmitTxFailure> {
        self.submit_tx(tx).await.map_err(|_| SubmitTxFailure)
    }
}
