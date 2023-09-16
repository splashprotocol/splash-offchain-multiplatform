use cml_chain::transaction::Transaction;

use cardano_submit_api::client::{Error, LocalTxSubmissionClient};
use spectrum_offchain::network::Network;

pub struct TxSubmit<const EraId: u16>(pub LocalTxSubmissionClient<EraId>);

#[async_trait::async_trait]
impl<const EraId: u16> Network<Transaction, Error> for TxSubmit<EraId> {
    async fn submit_tx(&mut self, tx: Transaction) -> Result<(), Error> {
        self.0.submit_tx(tx).await
    }
}
