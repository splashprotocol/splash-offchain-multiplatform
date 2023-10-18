use cml_chain::address::Address;
use cml_chain::builders::input_builder::InputBuilderResult;
use cml_crypto::PrivateKey;

use crate::data::ref_scripts::RefScriptsOutputs;

#[derive(Clone)]
pub struct ExecutionContext<'a> {
    pub operator_addr: Address,
    pub operator_prv: &'a PrivateKey,
    pub ref_scripts: RefScriptsOutputs,
    pub collateral: InputBuilderResult,
}

impl<'a> ExecutionContext<'a> {
    pub fn new(
        operator_addr: Address,
        operator_prv: &'a PrivateKey,
        ref_scripts: RefScriptsOutputs,
        collateral: InputBuilderResult,
    ) -> Self {
        ExecutionContext {
            operator_addr,
            operator_prv,
            ref_scripts,
            collateral,
        }
    }
}
