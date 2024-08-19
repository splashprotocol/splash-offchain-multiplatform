use cml_chain::Value;
use cml_core::ordered_hash_map::OrderedHashMap;
use linked_hash_map::Entry;

use crate::{AssetClass, Token};

pub trait ValueExtension {
    fn amount_of(&self, ac: AssetClass) -> Option<u64>;
    fn sub_unsafe(&mut self, ac: AssetClass, amt: u64);
    fn add_unsafe(&mut self, ac: AssetClass, amt: u64);
}

impl ValueExtension for Value {
    fn amount_of(&self, ac: AssetClass) -> Option<u64> {
        match ac {
            AssetClass::Native => Some(self.coin),
            AssetClass::Token(Token(policy, an)) => self.multiasset.get(&policy, &an.into()),
        }
    }

    fn sub_unsafe(&mut self, ac: AssetClass, amt: u64) {
        match ac {
            AssetClass::Native => self.coin = self.coin - amt,
            AssetClass::Token(Token(policy, an)) => {
                if let Entry::Occupied(mut bundle) = self.multiasset.entry(policy) {
                    if let Entry::Occupied(mut asset) = bundle.get_mut().entry(an.into()) {
                        let value = *asset.get();
                        if value > amt {
                            asset.insert(value - amt);
                        } else {
                            asset.remove();
                        }
                    }
                    let bundle_is_empty = bundle.get().is_empty();
                    if bundle_is_empty {
                        bundle.remove();
                    }
                }
            }
        }
    }

    fn add_unsafe(&mut self, ac: AssetClass, amt: u64) {
        match ac {
            AssetClass::Native => self.coin = self.coin + amt,
            AssetClass::Token(Token(policy, an)) => match self.multiasset.entry(policy) {
                Entry::Occupied(mut bundle) => {
                    if let Some(asset) = bundle.get_mut().get_mut(&an.into()) {
                        *asset += amt;
                    } else {
                        bundle.get_mut().insert(an.into(), amt);
                    }
                }
                Entry::Vacant(bundle) => {
                    let mut inner = OrderedHashMap::new();
                    inner.insert(an.into(), amt);
                    bundle.insert(inner);
                }
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use cml_chain::assets::MultiAsset;
    use cml_chain::{PolicyId, Value};

    use crate::value::ValueExtension;
    use crate::{AssetClass, AssetName, Token};

    #[test]
    fn add_subtract_token() {
        let ac1 = AssetClass::Token(Token(PolicyId::from([1u8; 28]), AssetName::from((32, [1u8; 32]))));
        let ac1_amt = 100;
        let ac2 = AssetClass::Token(Token(PolicyId::from([2u8; 28]), AssetName::from((32, [2u8; 32]))));
        let ac2_amt = 200;
        let ac3 = AssetClass::Token(Token(PolicyId::from([2u8; 28]), AssetName::from((32, [3u8; 32]))));
        let ac3_amt = 300;
        let mut value = Value::new(1, MultiAsset::new());
        value.add_unsafe(ac1, ac1_amt);
        assert_eq!(value.amount_of(ac1), Some(ac1_amt));
        value.add_unsafe(ac2, ac2_amt);
        value.add_unsafe(ac3, ac3_amt);
        assert_eq!(value.amount_of(ac3), Some(ac3_amt));
        let ac3_amt_sub = 200;
        value.sub_unsafe(ac3, ac3_amt_sub);
        assert_eq!(value.amount_of(ac3), Some(ac3_amt - ac3_amt_sub));
        let ac3_amt_sub = 100;
        value.sub_unsafe(ac3, ac3_amt_sub);
        assert_eq!(value.amount_of(ac3), None);
        assert_eq!(value.amount_of(ac2), Some(ac2_amt));
        value.sub_unsafe(ac2, ac2_amt);
        assert_eq!(value.amount_of(ac2), None);
        assert_eq!(value.amount_of(ac1), Some(ac1_amt));
        value.sub_unsafe(ac1, ac1_amt);
        assert_eq!(value.amount_of(ac1), None);
        assert!(value.multiasset.is_empty());
    }
}
