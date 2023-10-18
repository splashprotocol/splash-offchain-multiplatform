use crate::data::ref_scripts::RefScriptsOutputs;
use cml_chain::builders::input_builder::InputBuilderResult;
use cml_crypto::{Ed25519KeyHash, PrivateKey};

#[derive(Clone)]
pub struct OrderExecutionContext<'a> {
    pub batcher_pkh: Ed25519KeyHash,
    pub batcher_private: &'a PrivateKey,
    pub ref_scripts: RefScriptsOutputs,
    pub collateral: InputBuilderResult,
}

impl<'a> OrderExecutionContext<'a> {
    pub fn new(
        batcher_pkh: Ed25519KeyHash,
        batcher_private: &'a PrivateKey,
        ref_scripts: RefScriptsOutputs,
        collateral: InputBuilderResult,
    ) -> Self {
        OrderExecutionContext {
            batcher_pkh,
            batcher_private,
            ref_scripts,
            collateral,
        }
    }
}
