use cml_chain::utils::BigInt;
use spectrum_cardano_lib::plutus_data::IntoPlutusData;
use spectrum_offchain::data::{EntitySnapshot, Identifier, Stable};

#[derive(Copy, Clone, PartialEq, Eq, Ord, PartialOrd, Debug, Hash, derive_more::Display)]
pub struct FarmId(u64);

impl Identifier for FarmId {
    type For = SmartFarm;
}

impl IntoPlutusData for FarmId {
    fn into_pd(self) -> cml_chain::plutus::PlutusData {
        cml_chain::plutus::PlutusData::new_integer(BigInt::from(self.0))
    }
}

pub struct SmartFarm {
    pub farm_id: FarmId,
}

impl Stable for SmartFarm {
    type StableId = FarmId;
    fn stable_id(&self) -> Self::StableId {
        self.farm_id
    }
}

impl EntitySnapshot for SmartFarm {
    type Version = u64;
    fn version(&self) -> Self::Version {
        todo!()
    }
}
