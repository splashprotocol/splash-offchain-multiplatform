use type_equalities::IsEqual;

use spectrum_cardano_lib::OutputRef;
use spectrum_offchain::data::Has;
use spectrum_offchain_cardano::creds::OperatorCred;
use spectrum_offchain_cardano::data::deposit::DepositOrderBounds;
use spectrum_offchain_cardano::data::redeem::RedeemOrderBounds;
use spectrum_offchain_cardano::deployment::{DeployedScriptInfo, ProtocolScriptHashes};
use spectrum_offchain_cardano::deployment::ProtocolValidator::{
    BalanceFnPoolDeposit, BalanceFnPoolRedeem, BalanceFnPoolV1, ConstFnPoolDeposit, ConstFnPoolFeeSwitch,
    ConstFnPoolFeeSwitchBiDirFee, ConstFnPoolRedeem, ConstFnPoolSwap, ConstFnPoolV1, ConstFnPoolV2,
    LimitOrderV1, LimitOrderWitnessV1,
};
use spectrum_offchain_cardano::utxo::ConsumedInputs;

use crate::bounds::Bounds;
use crate::orders::limit::LimitOrderBounds;

#[derive(Copy, Clone, Debug)]
pub struct HandlerContextProto {
    pub executor_cred: OperatorCred,
    pub scripts: ProtocolScriptHashes,
    pub bounds: Bounds,
}

#[derive(Copy, Clone, Debug)]
pub struct HandlerContext {
    pub output_ref: OutputRef,
    pub consumed_utxos: ConsumedInputs,
    pub executor_cred: OperatorCred,
    pub scripts: ProtocolScriptHashes,
    pub bounds: Bounds,
}

impl Has<LimitOrderBounds> for HandlerContext {
    fn select<U: IsEqual<LimitOrderBounds>>(&self) -> LimitOrderBounds {
        self.bounds.limit_order
    }
}

impl Has<DepositOrderBounds> for HandlerContext {
    fn select<U: IsEqual<DepositOrderBounds>>(&self) -> DepositOrderBounds {
        self.bounds.deposit_order
    }
}

impl Has<RedeemOrderBounds> for HandlerContext {
    fn select<U: IsEqual<RedeemOrderBounds>>(&self) -> RedeemOrderBounds {
        self.bounds.redeem_order
    }
}

impl Has<ConsumedInputs> for HandlerContext {
    fn select<U: IsEqual<ConsumedInputs>>(&self) -> ConsumedInputs {
        self.consumed_utxos
    }
}

impl Has<DeployedScriptInfo<{ ConstFnPoolV1 as u8 }>> for HandlerContext {
    fn select<U: IsEqual<DeployedScriptInfo<{ ConstFnPoolV1 as u8 }>>>(
        &self,
    ) -> DeployedScriptInfo<{ ConstFnPoolV1 as u8 }> {
        self.scripts.const_fn_pool_v1.clone()
    }
}

impl Has<DeployedScriptInfo<{ ConstFnPoolV2 as u8 }>> for HandlerContext {
    fn select<U: IsEqual<DeployedScriptInfo<{ ConstFnPoolV2 as u8 }>>>(
        &self,
    ) -> DeployedScriptInfo<{ ConstFnPoolV2 as u8 }> {
        self.scripts.const_fn_pool_v2.clone()
    }
}

impl Has<DeployedScriptInfo<{ ConstFnPoolFeeSwitch as u8 }>> for HandlerContext {
    fn select<U: IsEqual<DeployedScriptInfo<{ ConstFnPoolFeeSwitch as u8 }>>>(
        &self,
    ) -> DeployedScriptInfo<{ ConstFnPoolFeeSwitch as u8 }> {
        self.scripts.const_fn_pool_fee_switch.clone()
    }
}

impl Has<DeployedScriptInfo<{ ConstFnPoolFeeSwitchBiDirFee as u8 }>> for HandlerContext {
    fn select<U: IsEqual<DeployedScriptInfo<{ ConstFnPoolFeeSwitchBiDirFee as u8 }>>>(
        &self,
    ) -> DeployedScriptInfo<{ ConstFnPoolFeeSwitchBiDirFee as u8 }> {
        self.scripts.const_fn_pool_fee_switch_bidir_fee.clone()
    }
}

impl Has<DeployedScriptInfo<{ ConstFnPoolSwap as u8 }>> for HandlerContext {
    fn select<U: IsEqual<DeployedScriptInfo<{ ConstFnPoolSwap as u8 }>>>(
        &self,
    ) -> DeployedScriptInfo<{ ConstFnPoolSwap as u8 }> {
        self.scripts.const_fn_pool_swap.clone()
    }
}

impl Has<DeployedScriptInfo<{ ConstFnPoolDeposit as u8 }>> for HandlerContext {
    fn select<U: IsEqual<DeployedScriptInfo<{ ConstFnPoolDeposit as u8 }>>>(
        &self,
    ) -> DeployedScriptInfo<{ ConstFnPoolDeposit as u8 }> {
        self.scripts.const_fn_pool_deposit.clone()
    }
}

impl Has<DeployedScriptInfo<{ ConstFnPoolRedeem as u8 }>> for HandlerContext {
    fn select<U: IsEqual<DeployedScriptInfo<{ ConstFnPoolRedeem as u8 }>>>(
        &self,
    ) -> DeployedScriptInfo<{ ConstFnPoolRedeem as u8 }> {
        self.scripts.const_fn_pool_redeem.clone()
    }
}

impl Has<DeployedScriptInfo<{ BalanceFnPoolV1 as u8 }>> for HandlerContext {
    fn select<U: IsEqual<DeployedScriptInfo<{ BalanceFnPoolV1 as u8 }>>>(
        &self,
    ) -> DeployedScriptInfo<{ BalanceFnPoolV1 as u8 }> {
        self.scripts.balance_fn_pool_v1.clone()
    }
}

impl Has<DeployedScriptInfo<{ BalanceFnPoolRedeem as u8 }>> for HandlerContext {
    fn select<U: IsEqual<DeployedScriptInfo<{ BalanceFnPoolRedeem as u8 }>>>(
        &self,
    ) -> DeployedScriptInfo<{ BalanceFnPoolRedeem as u8 }> {
        self.scripts.balance_fn_pool_redeem.clone()
    }
}

impl Has<DeployedScriptInfo<{ BalanceFnPoolDeposit as u8 }>> for HandlerContext {
    fn select<U: IsEqual<DeployedScriptInfo<{ BalanceFnPoolDeposit as u8 }>>>(
        &self,
    ) -> DeployedScriptInfo<{ BalanceFnPoolDeposit as u8 }> {
        self.scripts.balance_fn_pool_deposit.clone()
    }
}

impl Has<DeployedScriptInfo<{ LimitOrderV1 as u8 }>> for HandlerContext {
    fn select<U: IsEqual<DeployedScriptInfo<{ LimitOrderV1 as u8 }>>>(
        &self,
    ) -> DeployedScriptInfo<{ LimitOrderV1 as u8 }> {
        self.scripts.limit_order.clone()
    }
}

impl Has<DeployedScriptInfo<{ LimitOrderWitnessV1 as u8 }>> for HandlerContext {
    fn select<U: IsEqual<DeployedScriptInfo<{ LimitOrderWitnessV1 as u8 }>>>(
        &self,
    ) -> DeployedScriptInfo<{ LimitOrderWitnessV1 as u8 }> {
        self.scripts.limit_order_witness.clone()
    }
}

impl HandlerContext {
    pub fn new(
        output_ref: OutputRef,
        consumed_utxos: ConsumedInputs,
        prototype: HandlerContextProto,
    ) -> Self {
        Self {
            output_ref,
            consumed_utxos,
            executor_cred: prototype.executor_cred,
            scripts: prototype.scripts,
            bounds: prototype.bounds,
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
