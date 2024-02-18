use cml_chain::address::Address;
use spectrum_cardano_lib::collateral::Collateral;

use crate::data::ref_scripts::ReferenceOutputs;

#[derive(Clone)]
pub struct ExecutionContext {
    pub operator_addr: Address,
    pub ref_scripts: ReferenceOutputs,
    pub collateral: Collateral,
    pub network_id: u8,
}

impl ExecutionContext {
    pub fn new(
        operator_addr: Address,
        ref_scripts: ReferenceOutputs,
        collateral: Collateral,
        network_id: u8,
    ) -> Self {
        ExecutionContext {
            operator_addr,
            ref_scripts,
            collateral,
            network_id,
        }
    }
}
