use cml_chain::builders::tx_builder::TransactionUnspentOutput;

use crate::data::deposit::ClassicalOnChainDeposit;
use crate::data::redeem::ClassicalOnChainRedeem;

#[derive(Debug, Clone)]
pub struct ReferenceOutputs {
    pub pool_v1: TransactionUnspentOutput,
    pub pool_v2: TransactionUnspentOutput,
    pub fee_switch_pool: TransactionUnspentOutput,
    pub fee_switch_pool_bidir_fee: TransactionUnspentOutput,
    pub balance_pool: TransactionUnspentOutput,
    pub swap: TransactionUnspentOutput,
    pub deposit: TransactionUnspentOutput,
    pub redeem: TransactionUnspentOutput,
    pub spot_order: TransactionUnspentOutput,
    pub spot_order_batch_validator: TransactionUnspentOutput,
}

pub trait RequiresRefScript {
    fn get_ref_script(self, ref_scripts: ReferenceOutputs) -> TransactionUnspentOutput;
}

impl RequiresRefScript for ClassicalOnChainDeposit {
    fn get_ref_script(self, ref_scripts: ReferenceOutputs) -> TransactionUnspentOutput {
        ref_scripts.deposit
    }
}

impl RequiresRefScript for ClassicalOnChainRedeem {
    fn get_ref_script(self, ref_scripts: ReferenceOutputs) -> TransactionUnspentOutput {
        ref_scripts.redeem
    }
}
