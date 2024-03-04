use spectrum_offchain::data::{EntitySnapshot, Stable};

pub struct SmartFarm {}

impl Stable for SmartFarm {
    type StableId = u64;
    fn stable_id(&self) -> Self::StableId {
        todo!()
    }
}

impl EntitySnapshot for SmartFarm {
    type Version = u64;
    fn version(&self) -> Self::Version {
        todo!()
    }
}
