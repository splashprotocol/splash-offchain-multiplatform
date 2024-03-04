use spectrum_cardano_lib::Token;
use spectrum_offchain::data::{EntitySnapshot, Identifier, Stable};

use crate::GenesisEpochStartTime;
use crate::time::{epoch_end, NetworkTime, ProtocolEpoch};

#[derive(Copy, Clone, PartialEq, Eq, Ord, PartialOrd)]
pub struct InflationBoxId(Token);

impl Identifier for InflationBoxId {
    type For = InflationBox;
}

#[derive(Copy, Clone, Debug)]
pub struct InflationBox {
    pub last_processed_epoch: ProtocolEpoch,
}

impl InflationBox {
    pub fn active_epoch(&self, genesis: GenesisEpochStartTime, now: NetworkTime) -> ProtocolEpoch {
        if epoch_end(genesis, self.last_processed_epoch) < now {
            self.last_processed_epoch
        } else {
            self.last_processed_epoch + 1
        }
    }
}

impl Stable for InflationBox {
    type StableId = u64;
    fn stable_id(&self) -> Self::StableId {
        todo!()
    }
}

impl EntitySnapshot for InflationBox {
    type Version = u64;
    fn version(&self) -> Self::Version {
        todo!()
    }
}
