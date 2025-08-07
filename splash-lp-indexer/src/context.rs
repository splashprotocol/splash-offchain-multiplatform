use cml_crypto::ScriptHash;
use spectrum_offchain::domain::Has;
use spectrum_offchain_cardano::data::pool::PoolValidation;
use spectrum_offchain_cardano::deployment::ProtocolValidator::*;
use spectrum_offchain_cardano::deployment::{DeployedScriptInfo, ProtocolDeployment as DexDeployment};
use splash_dao_offchain::deployment::ProtocolValidator::*;
use splash_dao_offchain::deployment::{ProtocolDeployment as DaoDeployment, ProtocolTokens as DaoTokens};
use splash_dao_offchain::protocol_config::{
    BufferWalletScript, FarmAuthPolicy, PermManagerAuthPolicy, SplashPolicy, WPFactoryAuthPolicy,
};
use splash_reward_distributor::config::HarvestLimits;
use type_equalities::IsEqual;

pub struct Context {
    pub dex_deployment: DexDeployment,
    pub dao_deployment: DaoDeployment,
    pub dao_tokens: DaoTokens,
    pub pool_validation: PoolValidation,
    pub harvest_limits: HarvestLimits,
    pub splash_policy_id: ScriptHash,
}

impl Has<DeployedScriptInfo<{ WpFactory as u8 }>> for Context {
    fn select<U: IsEqual<DeployedScriptInfo<{ WpFactory as u8 }>>>(
        &self,
    ) -> DeployedScriptInfo<{ WpFactory as u8 }> {
        DeployedScriptInfo::from(&self.dao_deployment.wp_factory)
    }
}

impl Has<DeployedScriptInfo<{ SmartFarm as u8 }>> for Context {
    fn select<U: IsEqual<DeployedScriptInfo<{ SmartFarm as u8 }>>>(
        &self,
    ) -> DeployedScriptInfo<{ SmartFarm as u8 }> {
        (&self.dao_deployment.smart_farm).into()
    }
}

impl Has<DeployedScriptInfo<{ PermManager as u8 }>> for Context {
    fn select<U: IsEqual<DeployedScriptInfo<{ PermManager as u8 }>>>(
        &self,
    ) -> DeployedScriptInfo<{ PermManager as u8 }> {
        (&self.dao_deployment.perm_manager).into()
    }
}

impl Has<SplashPolicy> for Context {
    fn select<U: IsEqual<SplashPolicy>>(&self) -> SplashPolicy {
        SplashPolicy(self.splash_policy_id)
    }
}

impl Has<PermManagerAuthPolicy> for Context {
    fn select<U: IsEqual<PermManagerAuthPolicy>>(&self) -> PermManagerAuthPolicy {
        PermManagerAuthPolicy(self.dao_tokens.perm_auth.policy_id)
    }
}

impl Has<WPFactoryAuthPolicy> for Context {
    fn select<U: IsEqual<WPFactoryAuthPolicy>>(&self) -> WPFactoryAuthPolicy {
        WPFactoryAuthPolicy(self.dao_tokens.wp_factory_auth.policy_id)
    }
}

impl Has<FarmAuthPolicy> for Context {
    fn select<U: IsEqual<FarmAuthPolicy>>(&self) -> FarmAuthPolicy {
        FarmAuthPolicy(self.dao_deployment.smart_farm.hash)
    }
}

impl Has<DeployedScriptInfo<{ ConstFnPoolV1 as u8 }>> for Context {
    fn select<U: IsEqual<DeployedScriptInfo<{ ConstFnPoolV1 as u8 }>>>(
        &self,
    ) -> DeployedScriptInfo<{ ConstFnPoolV1 as u8 }> {
        (&self.dex_deployment.const_fn_pool_v1).into()
    }
}

impl Has<DeployedScriptInfo<{ ConstFnPoolV2 as u8 }>> for Context {
    fn select<U: IsEqual<DeployedScriptInfo<{ ConstFnPoolV2 as u8 }>>>(
        &self,
    ) -> DeployedScriptInfo<{ ConstFnPoolV2 as u8 }> {
        (&self.dex_deployment.const_fn_pool_v2).into()
    }
}

impl Has<DeployedScriptInfo<{ ConstFnPoolFeeSwitch as u8 }>> for Context {
    fn select<U: IsEqual<DeployedScriptInfo<{ ConstFnPoolFeeSwitch as u8 }>>>(
        &self,
    ) -> DeployedScriptInfo<{ ConstFnPoolFeeSwitch as u8 }> {
        (&self.dex_deployment.const_fn_pool_fee_switch).into()
    }
}

impl Has<DeployedScriptInfo<{ ConstFnPoolFeeSwitchV2 as u8 }>> for Context {
    fn select<U: IsEqual<DeployedScriptInfo<{ ConstFnPoolFeeSwitchV2 as u8 }>>>(
        &self,
    ) -> DeployedScriptInfo<{ ConstFnPoolFeeSwitchV2 as u8 }> {
        (&self.dex_deployment.const_fn_pool_fee_switch_v2).into()
    }
}

impl Has<DeployedScriptInfo<{ ConstFnPoolFeeSwitchBiDirFee as u8 }>> for Context {
    fn select<U: IsEqual<DeployedScriptInfo<{ ConstFnPoolFeeSwitchBiDirFee as u8 }>>>(
        &self,
    ) -> DeployedScriptInfo<{ ConstFnPoolFeeSwitchBiDirFee as u8 }> {
        (&self.dex_deployment.const_fn_pool_fee_switch_bidir_fee).into()
    }
}

impl Has<DeployedScriptInfo<{ BalanceFnPoolV1 as u8 }>> for Context {
    fn select<U: IsEqual<DeployedScriptInfo<{ BalanceFnPoolV1 as u8 }>>>(
        &self,
    ) -> DeployedScriptInfo<{ BalanceFnPoolV1 as u8 }> {
        (&self.dex_deployment.balance_fn_pool_v1).into()
    }
}

impl Has<DeployedScriptInfo<{ BalanceFnPoolV2 as u8 }>> for Context {
    fn select<U: IsEqual<DeployedScriptInfo<{ BalanceFnPoolV2 as u8 }>>>(
        &self,
    ) -> DeployedScriptInfo<{ BalanceFnPoolV2 as u8 }> {
        (&self.dex_deployment.balance_fn_pool_v2).into()
    }
}

impl Has<DeployedScriptInfo<{ StableFnPoolT2T as u8 }>> for Context {
    fn select<U: IsEqual<DeployedScriptInfo<{ StableFnPoolT2T as u8 }>>>(
        &self,
    ) -> DeployedScriptInfo<{ StableFnPoolT2T as u8 }> {
        (&self.dex_deployment.stable_fn_pool_t2t).into()
    }
}

impl Has<DeployedScriptInfo<{ RoyaltyPoolV1 as u8 }>> for Context {
    fn select<U: IsEqual<DeployedScriptInfo<{ RoyaltyPoolV1 as u8 }>>>(
        &self,
    ) -> DeployedScriptInfo<{ RoyaltyPoolV1 as u8 }> {
        (&self.dex_deployment.royalty_pool).into()
    }
}

impl Has<PoolValidation> for Context {
    fn select<U: IsEqual<PoolValidation>>(&self) -> PoolValidation {
        self.pool_validation.clone()
    }
}

impl Has<DeployedScriptInfo<{ HarvestOrder as u8 }>> for Context {
    fn select<U: IsEqual<DeployedScriptInfo<{ HarvestOrder as u8 }>>>(
        &self,
    ) -> DeployedScriptInfo<{ HarvestOrder as u8 }> {
        (&self.dao_deployment.harvest_order).into()
    }
}

impl Has<HarvestLimits> for Context {
    fn select<U: IsEqual<HarvestLimits>>(&self) -> HarvestLimits {
        self.harvest_limits
    }
}

impl Has<BufferWalletScript> for Context {
    fn select<U: IsEqual<BufferWalletScript>>(&self) -> BufferWalletScript {
        BufferWalletScript(self.dao_deployment.buffer_wallet.clone())
    }
}
