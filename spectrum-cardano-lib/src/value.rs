use cml_chain::Value;

use crate::AssetClass;

pub trait ValueExtension {
    fn amount_of(&self, ac: AssetClass) -> Option<u64>;
}

impl ValueExtension for Value {
    fn amount_of(&self, ac: AssetClass) -> Option<u64> {
        match ac {
            AssetClass::Native => Some(self.coin),
            AssetClass::Token((policy, an)) => self.multiasset.get(&policy, &an.into()),
        }
    }
}
