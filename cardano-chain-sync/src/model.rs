use cml_chain::block::Block;
use pallas_network::miniprotocols::Point;

#[derive(Clone)]
pub enum ChainUpgrade {
    RollForward(Block),
    RollBackward(Point),
}
