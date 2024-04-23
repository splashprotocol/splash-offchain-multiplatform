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
    RollForward {
        blk: Block,
        blk_bytes: Vec<u8>,
        replayed: bool,
    },
    RollBackward(Point),
}
