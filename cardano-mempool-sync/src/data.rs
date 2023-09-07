#[derive(Debug, Clone)]
pub enum MempoolUpdate<Tx> {
    TxAccepted(Tx),
}
