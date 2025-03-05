use crate::client::Point;
use cml_crypto::BlockHeaderHash;

#[derive(Clone)]
pub enum LedgerBlockEvent<Block> {
    RollForward(Block),
    RollBackward(Block),
}

#[derive(Clone, Debug)]
pub enum LedgerTxEvent<Tx> {
    TxApplied {
        tx: Tx,
        slot: u64,
        block_number: u64,
        block_hash: BlockHeaderHash,
    },
    TxUnapplied {
        tx: Tx,
        slot: u64,
        block_number: u64,
        block_hash: BlockHeaderHash,
    },
}

impl<Tx> LedgerTxEvent<Tx> {
    pub fn map<U, F>(self, f: F) -> LedgerTxEvent<U>
    where
        F: FnOnce(Tx) -> U,
    {
        match self {
            LedgerTxEvent::TxApplied {
                tx,
                slot,
                block_number,
                block_hash,
            } => LedgerTxEvent::TxApplied {
                tx: f(tx),
                slot,
                block_number,
                block_hash,
            },
            LedgerTxEvent::TxUnapplied {
                tx,
                slot,
                block_number,
                block_hash,
            } => LedgerTxEvent::TxUnapplied {
                tx: f(tx),
                slot,
                block_number,
                block_hash,
            },
        }
    }
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
