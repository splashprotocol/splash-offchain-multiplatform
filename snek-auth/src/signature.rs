use crate::AuthRequest;
use cml_crypto::blake2b256;
use spectrum_cardano_lib::{AssetClass, OutputRef};
use std::ops::Add;

#[derive(serde::Serialize)]
pub struct DataToSign {
    input_oref: OutputRef,
    order_index: u64,
    input_amount: u64,
    input_asset: AssetClass,
    output_asset: AssetClass,
}

impl From<AuthRequest> for DataToSign {
    fn from(value: AuthRequest) -> Self {
        Self {
            input_oref: value.input_oref,
            order_index: value.order_index,
            input_amount: value.input_amount,
            input_asset: value.input_asset,
            output_asset: value.output_asset,
        }
    }
}

#[derive(Clone)]
pub struct Signature {
    secret: String,
}

impl Signature {
    pub fn new(secret: String) -> Signature {
        Self { secret }
    }

    pub fn verify(&self, to_sign: DataToSign, signature: String) -> Option<bool> {
        let raw_value = serde_json::to_string(&to_sign).ok().unwrap();
        Some(
            hex::encode(blake2b256(
                hex::encode(raw_value.add(self.secret.as_str()))
                    .into_bytes()
                    .as_ref(),
            )) == signature,
        )
    }
}
