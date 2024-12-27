use crate::client::Point;

#[derive(Clone)]
pub enum LedgerBlockEvent<Block> {
    RollForward(Block),
    RollBackward(Block),
}

impl<Block> LedgerBlockEvent<Block> {
    pub fn map<T2, F>(self, f: F) -> LedgerBlockEvent<T2>
    where
        F: FnOnce(Block) -> T2,
    {
        match self {
            LedgerBlockEvent::RollForward(blk) => LedgerBlockEvent::RollForward(f(blk)),
            LedgerBlockEvent::RollBackward(blk) => LedgerBlockEvent::RollBackward(f(blk)),
        }
    }
}

#[derive(Clone, Debug)]
pub enum LedgerTxEvent<Tx> {
    TxApplied { tx: Tx, slot: u64, block_number: u64 },
    TxUnapplied { tx: Tx, slot: u64, block_number: u64 },
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
