use cml_chain::address::EnterpriseAddress;
use cml_crypto::{Ed25519KeyHash, ScriptHash};
use spectrum_cardano_lib::NetworkId;
use spectrum_offchain::domain::Has;
use spectrum_offchain_cardano::data::pool::PoolValidation;
use spectrum_offchain_cardano::deployment::ProtocolValidator::*;
use spectrum_offchain_cardano::deployment::{DeployedScriptInfo, ProtocolDeployment as DexDeployment};
use splash_dao_offchain::deployment::ProtocolValidator::*;
use splash_dao_offchain::deployment::{ProtocolDeployment as DaoDeployment, ProtocolTokens as DaoTokens};
use splash_dao_offchain::protocol_config::{
    BufferWalletScript, FarmAuthPolicy, OperatorCreds, PermManagerAuthPolicy, SplashPolicy,
    WPFactoryAuthPolicy,
};
use type_equalities::IsEqual;
use crate::config::HarvestLimits;

pub struct Context {
    pub dex_deployment: DexDeployment,
    pub dao_deployment: DaoDeployment,
    pub dao_tokens: DaoTokens,
    pub pool_validation: PoolValidation,
    pub harvest_limits: HarvestLimits,
    pub splash_policy_id: ScriptHash,
    pub network_id: NetworkId,
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

impl Has<NetworkId> for Context {
    fn select<U: IsEqual<NetworkId>>(&self) -> NetworkId {
        self.network_id
    }
}

impl Has<OperatorCreds> for Context {
    fn select<U: IsEqual<OperatorCreds>>(&self) -> OperatorCreds {
        // Need this since we use existing code in `splash-reward-distributor` to parse
        // `HarvestOrder`s (the reward bot uses these credentials to work with funding UTxOs).
        let dummy_key_hash = Ed25519KeyHash::from([0_u8; 28]);
        let dummy_address = cml_chain::address::Address::Enterprise(EnterpriseAddress::new(
            self.network_id.into(),
            cml_chain::certs::Credential::new_pub_key(dummy_key_hash),
        ));
        OperatorCreds(dummy_key_hash, dummy_address)
    }
}
