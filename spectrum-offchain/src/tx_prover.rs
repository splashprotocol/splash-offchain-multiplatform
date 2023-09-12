pub trait TxProver<TxCandidate, Tx> {
    fn prove(&self, candidate: TxCandidate) -> Tx;
}
