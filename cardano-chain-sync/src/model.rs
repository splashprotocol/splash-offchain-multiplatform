use cardano_multiplatform_lib::Block;
use pallas_network::miniprotocols::Point;

#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub enum ChainUpgrade {
    RollForward(Block),
    RollBackward(Point),
}
