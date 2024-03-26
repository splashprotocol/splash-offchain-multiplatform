use crate::client::Point;

#[derive(Clone)]
pub enum LedgerBlockEvent<Block> {
    RollForward(Block),
    RollBackward(Block),
}

#[derive(Clone, Debug)]
pub enum LedgerTxEvent<Tx> {
    TxApplied { tx: Tx, slot: u64 },
    TxUnapplied(Tx),
}

#[derive(Clone)]
pub enum ChainUpgrade<Block> {
    /// Deserialized block and it's serialized representation.
    RollForward(Block, Vec<u8>),
    RollBackward(Point),
}
