#[derive(Debug, Clone)]
pub enum MempoolUpdate<Tx> {
    TxAccepted(Tx),
    TxDropped(Tx),
}

impl<Tx> MempoolUpdate<Tx> {
    pub fn map<B, F>(self, f: F) -> MempoolUpdate<B>
    where
        F: FnOnce(Tx) -> B,
    {
        match self {
            MempoolUpdate::TxAccepted(tx) => MempoolUpdate::TxAccepted(f(tx)),
            MempoolUpdate::TxDropped(tx) => MempoolUpdate::TxDropped(f(tx)),
        }
    }
}
