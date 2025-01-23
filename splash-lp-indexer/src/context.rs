use spectrum_offchain::domain::Has;
use spectrum_offchain_cardano::data::pool::PoolValidation;
use spectrum_offchain_cardano::deployment::ProtocolValidator::*;
use spectrum_offchain_cardano::deployment::{DeployedScriptInfo, ProtocolDeployment};
use type_equalities::IsEqual;

pub struct Context {
    pub deployment: ProtocolDeployment,
    pub pool_validation: PoolValidation,
}

impl Has<DeployedScriptInfo<{ ConstFnPoolV1 as u8 }>> for Context {
    fn select<U: IsEqual<DeployedScriptInfo<{ ConstFnPoolV1 as u8 }>>>(
        &self,
    ) -> DeployedScriptInfo<{ ConstFnPoolV1 as u8 }> {
        (&self.deployment.const_fn_pool_v1).into()
    }
}

impl Has<DeployedScriptInfo<{ ConstFnPoolV2 as u8 }>> for Context {
    fn select<U: IsEqual<DeployedScriptInfo<{ ConstFnPoolV2 as u8 }>>>(
        &self,
    ) -> DeployedScriptInfo<{ ConstFnPoolV2 as u8 }> {
        (&self.deployment.const_fn_pool_v2).into()
    }
}

impl Has<DeployedScriptInfo<{ ConstFnPoolFeeSwitch as u8 }>> for Context {
    fn select<U: IsEqual<DeployedScriptInfo<{ ConstFnPoolFeeSwitch as u8 }>>>(
        &self,
    ) -> DeployedScriptInfo<{ ConstFnPoolFeeSwitch as u8 }> {
        (&self.deployment.const_fn_pool_fee_switch).into()
    }
}

impl Has<DeployedScriptInfo<{ ConstFnPoolFeeSwitchV2 as u8 }>> for Context {
    fn select<U: IsEqual<DeployedScriptInfo<{ ConstFnPoolFeeSwitchV2 as u8 }>>>(
        &self,
    ) -> DeployedScriptInfo<{ ConstFnPoolFeeSwitchV2 as u8 }> {
        (&self.deployment.const_fn_pool_fee_switch_v2).into()
    }
}

impl Has<DeployedScriptInfo<{ ConstFnPoolFeeSwitchBiDirFee as u8 }>> for Context {
    fn select<U: IsEqual<DeployedScriptInfo<{ ConstFnPoolFeeSwitchBiDirFee as u8 }>>>(
        &self,
    ) -> DeployedScriptInfo<{ ConstFnPoolFeeSwitchBiDirFee as u8 }> {
        (&self.deployment.const_fn_pool_fee_switch_bidir_fee).into()
    }
}

impl Has<DeployedScriptInfo<{ BalanceFnPoolV1 as u8 }>> for Context {
    fn select<U: IsEqual<DeployedScriptInfo<{ BalanceFnPoolV1 as u8 }>>>(
        &self,
    ) -> DeployedScriptInfo<{ BalanceFnPoolV1 as u8 }> {
        (&self.deployment.balance_fn_pool_v1).into()
    }
}

impl Has<DeployedScriptInfo<{ BalanceFnPoolV2 as u8 }>> for Context {
    fn select<U: IsEqual<DeployedScriptInfo<{ BalanceFnPoolV2 as u8 }>>>(
        &self,
    ) -> DeployedScriptInfo<{ BalanceFnPoolV2 as u8 }> {
        (&self.deployment.balance_fn_pool_v2).into()
    }
}

impl Has<DeployedScriptInfo<{ StableFnPoolT2T as u8 }>> for Context {
    fn select<U: IsEqual<DeployedScriptInfo<{ StableFnPoolT2T as u8 }>>>(
        &self,
    ) -> DeployedScriptInfo<{ StableFnPoolT2T as u8 }> {
        (&self.deployment.stable_fn_pool_t2t).into()
    }
}

impl Has<DeployedScriptInfo<{ RoyaltyPoolV1 as u8 }>> for Context {
    fn select<U: IsEqual<DeployedScriptInfo<{ RoyaltyPoolV1 as u8 }>>>(
        &self,
    ) -> DeployedScriptInfo<{ RoyaltyPoolV1 as u8 }> {
        (&self.deployment.royalty_pool).into()
    }
}

impl Has<PoolValidation> for Context {
    fn select<U: IsEqual<PoolValidation>>(&self) -> PoolValidation {
        self.pool_validation.clone()
    }
}
