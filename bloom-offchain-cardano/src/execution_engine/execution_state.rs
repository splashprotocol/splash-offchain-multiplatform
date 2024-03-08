use cml_chain::builders::tx_builder::TransactionBuilder;
use cml_chain::Value;

use spectrum_cardano_lib::protocol_params::constant_tx_builder;
use spectrum_cardano_lib::value::ValueExtension;
use spectrum_cardano_lib::AssetClass;

pub struct ExecutionState {
    pub tx_builder: TransactionBuilder,
    pub execution_budget_acc: Value,
}

impl ExecutionState {
    pub fn new() -> Self {
        Self {
            tx_builder: constant_tx_builder(),
            execution_budget_acc: Value::zero(),
        }
    }

    pub fn add_ex_budget(&mut self, ac: AssetClass, amount: u64) {
        self.execution_budget_acc.add_unsafe(ac.into(), amount);
    }
}
