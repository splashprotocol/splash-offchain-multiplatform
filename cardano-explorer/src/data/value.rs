use cml_chain::{PolicyId, Value as CMLValue};
use serde::Deserialize;
use spectrum_cardano_lib::value::ValueExtension;
use spectrum_cardano_lib::AssetClass::{Native, Token};
use spectrum_cardano_lib::AssetName;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Value(Vec<ValueEntity>);

impl Value {
    pub fn contains_only_ada(&self) -> bool {
        self.0.len() == 1
            && self
                .0
                .first()
                .map_or(false, |entity_info| entity_info.policy_id.is_empty())
    }

    // There is impossible to create output without ada, but we are not
    // parsing this Value directly from blockchain and couldn't guarantee that
    // parsed value will contain ada. Option?
    pub fn get_ada_qty(&self) -> u64 {
        self.0
            .iter()
            .find(|entity_info| entity_info.policy_id.is_empty())
            .map(|ada_info| (ada_info.js_quantity.clone()).parse::<u64>().unwrap())
            .unwrap()
    }
}

#[derive(Debug)]
pub struct ValueConvertingError;

impl TryInto<CMLValue> for Value {
    type Error = ValueConvertingError;

    fn try_into(self) -> Result<CMLValue, Self::Error> {
        let mut value = CMLValue::zero();
        self.0.iter().for_each(|entity| {
            if entity.name.is_empty() {
                value.add_unsafe(Native, entity.quantity);
            } else {
                let policy_id = PolicyId::from_hex(entity.policy_id.as_str()).unwrap();
                let token_name = AssetName::try_from(entity.name.clone()).unwrap();
                value.add_unsafe(Token((policy_id, token_name)), entity.quantity);
            }
        });
        Ok(value)
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ValueEntity {
    policy_id: String,
    name: String,
    quantity: u64,
    js_quantity: String,
}
