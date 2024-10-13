//! Code to manually compute a change output for a TX where the fee is explicitly set (here CML will
//! not compute the change output for us). This is necessary for some DAO TXs because CML improperly
//! computes an insufficient TX fee.
use std::collections::HashMap;

use cml_chain::{
    address::Address,
    assets::MultiAsset,
    builders::{
        input_builder::InputBuilderResult,
        output_builder::{SingleOutputBuilderResult, TransactionOutputBuilder},
    },
    Value,
};
use cml_crypto::ScriptHash;

pub trait CreateChangeOutput {
    fn add_input<T: Into<AdaValue>>(&mut self, input: T);
    fn add_output<T: Into<AdaValue>>(&mut self, input: T);
    fn create_change_output(self, fee: u64, change_address: Address) -> SingleOutputBuilderResult;
}

#[derive(Default)]
pub struct ChangeOutputCreator {
    pub input_coin: u64,
    pub output_coin: u64,
    pub tokens_in_inputs: HashMap<(ScriptHash, cml_chain::assets::AssetName), u64>,
}

impl CreateChangeOutput for ChangeOutputCreator {
    fn add_input<T: Into<AdaValue>>(&mut self, input: T) {
        let AdaValue { coin, tokens } = input.into();
        self.input_coin += coin;
        for Token {
            policy_id,
            asset_name,
            quantity,
        } in tokens
        {
            let existing_quantity = self.tokens_in_inputs.entry((policy_id, asset_name)).or_insert(0);
            *existing_quantity += quantity;
        }
    }

    fn add_output<T: Into<AdaValue>>(&mut self, output: T) {
        let AdaValue { coin, tokens } = output.into();
        self.output_coin += coin;
        for Token {
            policy_id,
            asset_name,
            quantity,
        } in tokens
        {
            let mut remove = false;
            if let Some(qty) = self.tokens_in_inputs.get_mut(&(policy_id, asset_name.clone())) {
                *qty -= quantity;
                if *qty == 0 {
                    remove = true;
                }
            }
            if remove {
                self.tokens_in_inputs.remove(&(policy_id, asset_name));
            }
        }
    }

    fn create_change_output(self, fee: u64, change_address: Address) -> SingleOutputBuilderResult {
        let change_output_coin = self.input_coin - self.output_coin - fee;
        let mut change_assets = MultiAsset::default();
        for ((policy_id, asset_name), quantity) in self.tokens_in_inputs {
            assert!(change_assets.set(policy_id, asset_name, quantity).is_none());
        }

        TransactionOutputBuilder::new()
            .with_address(change_address)
            .next()
            .unwrap()
            .with_value(Value::new(change_output_coin, change_assets))
            .build()
            .unwrap()
    }
}

pub struct AdaValue {
    coin: u64,
    tokens: Vec<Token>,
}

pub struct Token {
    pub policy_id: ScriptHash,
    pub asset_name: cml_chain::assets::AssetName,
    pub quantity: u64,
}

impl From<&SingleOutputBuilderResult> for AdaValue {
    fn from(value: &SingleOutputBuilderResult) -> Self {
        Self::from(value.output.amount())
    }
}

impl From<&InputBuilderResult> for AdaValue {
    fn from(value: &InputBuilderResult) -> Self {
        Self::from(value.utxo_info.amount())
    }
}

impl From<&Value> for AdaValue {
    fn from(value: &Value) -> Self {
        let coin = value.coin;
        let mut tokens = vec![];
        for (policy_id, bundle) in value.multiasset.iter() {
            for (name, quantity) in bundle.iter() {
                tokens.push(Token {
                    policy_id: *policy_id,
                    asset_name: name.clone(),
                    quantity: *quantity,
                });
            }
        }

        Self { coin, tokens }
    }
}
