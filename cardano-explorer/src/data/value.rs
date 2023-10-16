use cml_chain::{PolicyId, Value as CMLValue};
use serde::Deserialize;
use spectrum_cardano_lib::AssetClass::{Native, Token};
use spectrum_cardano_lib::AssetName;
use spectrum_cardano_lib::value::ValueExtension;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Value(Vec<ValueEntity>);

#[derive(Debug)]
pub struct ValueConvertingError;

impl TryInto<CMLValue> for Value {
    type Error = ValueConvertingError;

    fn try_into(self) -> Result<CMLValue, Self::Error> {
        let mut value = CMLValue::zero();
        self.0.iter().for_each(|entity|
           {
               if entity.name.is_empty() {
                   value.add_unsafe(Native, entity.quantity);
               } else {
                   let policy_id = PolicyId::from_hex(entity.clone().policy_id.as_str()).unwrap();
                   let token_name = AssetName::try_from(entity.name.clone()).unwrap();
                   value.add_unsafe(Token((policy_id, token_name)), entity.quantity);
               }
           },
        );
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