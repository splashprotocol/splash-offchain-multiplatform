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
pub(crate) enum ChainUpgrade<Block> {
    RollForward(Block),
    RollBackward(pallas_network::miniprotocols::Point),
}
