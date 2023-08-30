pub mod block;

use cardano_model::Block;

#[derive(Debug, Clone)]
pub enum ChainUpgrade {
    RollForward(Block),
    RollBackward(Block),
}
