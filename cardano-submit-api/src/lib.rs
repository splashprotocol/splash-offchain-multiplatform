use cml_chain::transaction::Transaction;

pub mod client;

pub struct SubmitTxFailure;

#[async_trait::async_trait]
pub trait SubmitTx {
    async fn submit(&mut self, tx: Transaction) -> Result<(), SubmitTxFailure>;
}
