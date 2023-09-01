use cml_multi_era::MultiEraBlock;
use pallas_network::miniprotocols::Point;

#[derive(Clone)]
pub enum ChainUpgrade {
    RollForward(MultiEraBlock),
    RollBackward(Point),
}
