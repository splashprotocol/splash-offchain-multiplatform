use cml_chain::block::Block;
use cml_chain::transaction::Transaction;
use pallas_network::miniprotocols::Point;

#[derive(Clone)]
pub enum ChainUpgrade {
    RollForward(Block),
    RollBackward(Point),
}

#[derive(Clone, Debug)]
pub enum LedgerTxEvent {
    TxApplied(Transaction),
    TxUnapplied(Transaction),
}
