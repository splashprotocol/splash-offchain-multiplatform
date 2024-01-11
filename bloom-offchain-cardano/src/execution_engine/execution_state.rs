use cml_chain::builders::tx_builder::TransactionBuilder;
use cml_chain::Value;

use spectrum_cardano_lib::protocol_params::constant_tx_builder;

pub struct ExecutionState {
    pub tx_builder: TransactionBuilder,
    pub executor_fee_acc: Value,
}

impl ExecutionState {
    pub fn new() -> Self {
        Self {
            tx_builder: constant_tx_builder(),
            executor_fee_acc: Value::zero(),
        }
    }
}
