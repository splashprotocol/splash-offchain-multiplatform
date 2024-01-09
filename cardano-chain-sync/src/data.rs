use crate::client::Point;

#[derive(Clone)]
pub enum ChainUpgrade<Block> {
    RollForward(Block),
    RollBackward(pallas_network::miniprotocols::Point),
}

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

pub trait Positioned {
    fn get_position(&self) -> Point;
}
