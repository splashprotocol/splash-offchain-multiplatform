use cml_chain::builders::tx_builder::{TransactionBuilderConfig, TransactionBuilderConfigBuilder};
use cml_chain::fees::LinearFee;
use cml_crypto::Ed25519KeyHash;

#[derive(Clone)]
pub struct OrderExecutionContext {
    //pub builder_cfg: TransactionBuilderConfig,
    pub batcher_pkh: Ed25519KeyHash,
}

impl OrderExecutionContext {
    pub fn new(batcher_pkh: Ed25519KeyHash) -> Self {
        OrderExecutionContext {
            //builder_cfg: TransactionBuilderConfigBuilder::new().fee_algo(LinearFee::new(0, 0)).build().unwrap(), //todo: change 0,0
            batcher_pkh,
        }
    }
}