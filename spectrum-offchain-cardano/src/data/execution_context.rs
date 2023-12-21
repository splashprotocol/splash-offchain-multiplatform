use cml_chain::address::Address;
use cml_chain::builders::input_builder::InputBuilderResult;

use crate::data::ref_scripts::ReferenceOutputs;

#[derive(Clone)]
pub struct ExecutionContext {
    pub operator_addr: Address,
    pub ref_scripts: ReferenceOutputs,
    pub collateral: InputBuilderResult,
}

impl ExecutionContext {
    pub fn new(
        operator_addr: Address,
        ref_scripts: ReferenceOutputs,
        collateral: InputBuilderResult,
    ) -> Self {
        ExecutionContext {
            operator_addr,
            ref_scripts,
            collateral,
        }
    }
}
