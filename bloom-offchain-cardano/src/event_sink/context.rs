use type_equalities::IsEqual;

use spectrum_cardano_lib::OutputRef;
use spectrum_offchain::data::Has;
use spectrum_offchain_cardano::creds::OperatorCred;
use spectrum_offchain_cardano::deployment::ProtocolValidator::{
    BalanceFnPoolDeposit, BalanceFnPoolRedeem, BalanceFnPoolV1, ConstFnPoolDeposit, ConstFnPoolFeeSwitch,
    ConstFnPoolFeeSwitchBiDirFee, ConstFnPoolRedeem, ConstFnPoolSwap, ConstFnPoolV1, ConstFnPoolV2,
    LimitOrderV1, LimitOrderWitnessV1,
};
use spectrum_offchain_cardano::deployment::{DeployedScriptHash, ProtocolScriptHashes};

#[derive(Copy, Clone, Debug)]
pub struct HandlerContextProto {
    pub executor_cred: OperatorCred,
    pub scripts: ProtocolScriptHashes,
}

#[derive(Copy, Clone, Debug)]
pub struct HandlerContext {
    pub output_ref: OutputRef,
    pub executor_cred: OperatorCred,
    pub scripts: ProtocolScriptHashes,
}

impl Has<DeployedScriptHash<{ ConstFnPoolV1 as u8 }>> for HandlerContext {
    fn select<U: IsEqual<DeployedScriptHash<{ ConstFnPoolV1 as u8 }>>>(
        &self,
    ) -> DeployedScriptHash<{ ConstFnPoolV1 as u8 }> {
        self.scripts.const_fn_pool_v1.clone()
    }
}

impl Has<DeployedScriptHash<{ ConstFnPoolV2 as u8 }>> for HandlerContext {
    fn select<U: IsEqual<DeployedScriptHash<{ ConstFnPoolV2 as u8 }>>>(
        &self,
    ) -> DeployedScriptHash<{ ConstFnPoolV2 as u8 }> {
        self.scripts.const_fn_pool_v2.clone()
    }
}

impl Has<DeployedScriptHash<{ ConstFnPoolFeeSwitch as u8 }>> for HandlerContext {
    fn select<U: IsEqual<DeployedScriptHash<{ ConstFnPoolFeeSwitch as u8 }>>>(
        &self,
    ) -> DeployedScriptHash<{ ConstFnPoolFeeSwitch as u8 }> {
        self.scripts.const_fn_pool_fee_switch.clone()
    }
}

impl Has<DeployedScriptHash<{ ConstFnPoolFeeSwitchBiDirFee as u8 }>> for HandlerContext {
    fn select<U: IsEqual<DeployedScriptHash<{ ConstFnPoolFeeSwitchBiDirFee as u8 }>>>(
        &self,
    ) -> DeployedScriptHash<{ ConstFnPoolFeeSwitchBiDirFee as u8 }> {
        self.scripts.const_fn_pool_fee_switch_bidir_fee.clone()
    }
}

impl Has<DeployedScriptHash<{ ConstFnPoolSwap as u8 }>> for HandlerContext {
    fn select<U: IsEqual<DeployedScriptHash<{ ConstFnPoolSwap as u8 }>>>(
        &self,
    ) -> DeployedScriptHash<{ ConstFnPoolSwap as u8 }> {
        self.scripts.const_fn_pool_swap.clone()
    }
}

impl Has<DeployedScriptHash<{ ConstFnPoolDeposit as u8 }>> for HandlerContext {
    fn select<U: IsEqual<DeployedScriptHash<{ ConstFnPoolDeposit as u8 }>>>(
        &self,
    ) -> DeployedScriptHash<{ ConstFnPoolDeposit as u8 }> {
        self.scripts.const_fn_pool_deposit.clone()
    }
}

impl Has<DeployedScriptHash<{ ConstFnPoolRedeem as u8 }>> for HandlerContext {
    fn select<U: IsEqual<DeployedScriptHash<{ ConstFnPoolRedeem as u8 }>>>(
        &self,
    ) -> DeployedScriptHash<{ ConstFnPoolRedeem as u8 }> {
        self.scripts.const_fn_pool_redeem.clone()
    }
}

impl Has<DeployedScriptHash<{ BalanceFnPoolV1 as u8 }>> for HandlerContext {
    fn select<U: IsEqual<DeployedScriptHash<{ BalanceFnPoolV1 as u8 }>>>(
        &self,
    ) -> DeployedScriptHash<{ BalanceFnPoolV1 as u8 }> {
        self.scripts.balance_fn_pool_v1.clone()
    }
}

impl Has<DeployedScriptHash<{ BalanceFnPoolRedeem as u8 }>> for HandlerContext {
    fn select<U: IsEqual<DeployedScriptHash<{ BalanceFnPoolRedeem as u8 }>>>(
        &self,
    ) -> DeployedScriptHash<{ BalanceFnPoolRedeem as u8 }> {
        self.scripts.balance_fn_pool_redeem.clone()
    }
}

impl Has<DeployedScriptHash<{ BalanceFnPoolDeposit as u8 }>> for HandlerContext {
    fn select<U: IsEqual<DeployedScriptHash<{ BalanceFnPoolDeposit as u8 }>>>(
        &self,
    ) -> DeployedScriptHash<{ BalanceFnPoolDeposit as u8 }> {
        self.scripts.balance_fn_pool_deposit.clone()
    }
}

impl Has<DeployedScriptHash<{ LimitOrderV1 as u8 }>> for HandlerContext {
    fn select<U: IsEqual<DeployedScriptHash<{ LimitOrderV1 as u8 }>>>(
        &self,
    ) -> DeployedScriptHash<{ LimitOrderV1 as u8 }> {
        self.scripts.limit_order.clone()
    }
}

impl Has<DeployedScriptHash<{ LimitOrderWitnessV1 as u8 }>> for HandlerContext {
    fn select<U: IsEqual<DeployedScriptHash<{ LimitOrderWitnessV1 as u8 }>>>(
        &self,
    ) -> DeployedScriptHash<{ LimitOrderWitnessV1 as u8 }> {
        self.scripts.limit_order_witness.clone()
    }
}

impl HandlerContext {
    pub fn new(output_ref: OutputRef, prototype: HandlerContextProto) -> Self {
        Self {
            output_ref,
            executor_cred: prototype.executor_cred,
            scripts: prototype.scripts,
        }
    }
}

impl Has<OutputRef> for HandlerContext {
    fn select<U: IsEqual<OutputRef>>(&self) -> OutputRef {
        self.output_ref
    }
}

impl Has<OperatorCred> for HandlerContext {
    fn select<U: IsEqual<OperatorCred>>(&self) -> OperatorCred {
        self.executor_cred
    }
}
