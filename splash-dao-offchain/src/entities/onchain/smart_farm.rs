use spectrum_offchain::data::{EntitySnapshot, Identifier, Stable};

#[derive(Copy, Clone, PartialEq, Eq, Ord, PartialOrd, Debug)]
pub struct FarmId(u64);

impl Identifier for FarmId {
    type For = SmartFarm;
}

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
