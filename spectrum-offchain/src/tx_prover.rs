// todo: remove
use cml_chain::crypto::Vkeywitness;
use cml_chain::transaction::TransactionBody;

pub trait TxProver<TxCandidate, Tx> {
    fn prove(&self, candidate: TxCandidate) -> Tx;

    fn add_signature(&self, candidate: TransactionBody) -> Vkeywitness;
}
