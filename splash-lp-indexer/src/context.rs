use crate::config::HarvestLimits;
use cml_chain::address::EnterpriseAddress;
use cml_crypto::{Ed25519KeyHash, ScriptHash};
use spectrum_cardano_lib::collateral::Collateral;
use spectrum_cardano_lib::NetworkId;
use spectrum_offchain::domain::Has;
use spectrum_offchain_cardano::data::pool::PoolValidation;
use spectrum_offchain_cardano::deployment::{DeployedScriptInfo, ProtocolDeployment as DexDeployment};
use spectrum_offchain_cardano::deployment::{DeployedValidator, ProtocolValidator::*};
use spectrum_offchain_cardano::{has_deployed_script_info, has_deployed_validator};
use splash_dao_offchain::deployment::ProtocolValidator::*;
use splash_dao_offchain::deployment::{ProtocolDeployment as DaoDeployment, ProtocolTokens as DaoTokens};
use splash_dao_offchain::protocol_config::{
    BufferWalletAuthPolicy, OperatorCreds, PermManagerAuthPolicy, SplashPolicy, WPFactoryAuthPolicy,
};
use splash_dao_offchain::GenesisEpochStartTime;
use splash_yf_offchain::settings::MinLovelacePerHarvest;
use type_equalities::IsEqual;

pub struct RuntimeContext {
    pub dex_deployment: DexDeployment,
    pub dao_deployment: DaoDeployment,
    pub dao_tokens: DaoTokens,
    pub pool_validation: PoolValidation,
    pub harvest_limits: HarvestLimits,
    pub splash_policy_id: ScriptHash,
    pub network_id: NetworkId,
    pub genesis_epoch_start_time: GenesisEpochStartTime,
}

impl Has<SplashPolicy> for RuntimeContext {
    fn select<U: IsEqual<SplashPolicy>>(&self) -> SplashPolicy {
        SplashPolicy(self.splash_policy_id)
    }
}

impl Has<PermManagerAuthPolicy> for RuntimeContext {
    fn select<U: IsEqual<PermManagerAuthPolicy>>(&self) -> PermManagerAuthPolicy {
        PermManagerAuthPolicy(self.dao_tokens.perm_auth.policy_id)
    }
}

impl Has<WPFactoryAuthPolicy> for RuntimeContext {
    fn select<U: IsEqual<WPFactoryAuthPolicy>>(&self) -> WPFactoryAuthPolicy {
        WPFactoryAuthPolicy(self.dao_tokens.wp_factory_auth.policy_id)
    }
}

impl Has<BufferWalletAuthPolicy> for RuntimeContext {
    fn select<U: IsEqual<BufferWalletAuthPolicy>>(&self) -> BufferWalletAuthPolicy {
        BufferWalletAuthPolicy(self.dao_tokens.buffer_wallet.policy_id)
    }
}

impl Has<DeployedScriptInfo<{ ConstFnPoolV1 as u8 }>> for RuntimeContext {
    fn select<U: IsEqual<DeployedScriptInfo<{ ConstFnPoolV1 as u8 }>>>(
        &self,
    ) -> DeployedScriptInfo<{ ConstFnPoolV1 as u8 }> {
        (&self.dex_deployment.const_fn_pool_v1).into()
    }
}

impl Has<DeployedScriptInfo<{ ConstFnPoolV2 as u8 }>> for RuntimeContext {
    fn select<U: IsEqual<DeployedScriptInfo<{ ConstFnPoolV2 as u8 }>>>(
        &self,
    ) -> DeployedScriptInfo<{ ConstFnPoolV2 as u8 }> {
        (&self.dex_deployment.const_fn_pool_v2).into()
    }
}

impl Has<DeployedScriptInfo<{ ConstFnPoolFeeSwitch as u8 }>> for RuntimeContext {
    fn select<U: IsEqual<DeployedScriptInfo<{ ConstFnPoolFeeSwitch as u8 }>>>(
        &self,
    ) -> DeployedScriptInfo<{ ConstFnPoolFeeSwitch as u8 }> {
        (&self.dex_deployment.const_fn_pool_fee_switch).into()
    }
}

impl Has<DeployedScriptInfo<{ ConstFnPoolFeeSwitchV2 as u8 }>> for RuntimeContext {
    fn select<U: IsEqual<DeployedScriptInfo<{ ConstFnPoolFeeSwitchV2 as u8 }>>>(
        &self,
    ) -> DeployedScriptInfo<{ ConstFnPoolFeeSwitchV2 as u8 }> {
        (&self.dex_deployment.const_fn_pool_fee_switch_v2).into()
    }
}

impl Has<DeployedScriptInfo<{ ConstFnPoolFeeSwitchBiDirFee as u8 }>> for RuntimeContext {
    fn select<U: IsEqual<DeployedScriptInfo<{ ConstFnPoolFeeSwitchBiDirFee as u8 }>>>(
        &self,
    ) -> DeployedScriptInfo<{ ConstFnPoolFeeSwitchBiDirFee as u8 }> {
        (&self.dex_deployment.const_fn_pool_fee_switch_bidir_fee).into()
    }
}

impl Has<DeployedScriptInfo<{ BalanceFnPoolV1 as u8 }>> for RuntimeContext {
    fn select<U: IsEqual<DeployedScriptInfo<{ BalanceFnPoolV1 as u8 }>>>(
        &self,
    ) -> DeployedScriptInfo<{ BalanceFnPoolV1 as u8 }> {
        (&self.dex_deployment.balance_fn_pool_v1).into()
    }
}

impl Has<DeployedScriptInfo<{ BalanceFnPoolV2 as u8 }>> for RuntimeContext {
    fn select<U: IsEqual<DeployedScriptInfo<{ BalanceFnPoolV2 as u8 }>>>(
        &self,
    ) -> DeployedScriptInfo<{ BalanceFnPoolV2 as u8 }> {
        (&self.dex_deployment.balance_fn_pool_v2).into()
    }
}

impl Has<DeployedScriptInfo<{ StableFnPoolT2T as u8 }>> for RuntimeContext {
    fn select<U: IsEqual<DeployedScriptInfo<{ StableFnPoolT2T as u8 }>>>(
        &self,
    ) -> DeployedScriptInfo<{ StableFnPoolT2T as u8 }> {
        (&self.dex_deployment.stable_fn_pool_t2t).into()
    }
}

impl Has<DeployedScriptInfo<{ RoyaltyPoolV1 as u8 }>> for RuntimeContext {
    fn select<U: IsEqual<DeployedScriptInfo<{ RoyaltyPoolV1 as u8 }>>>(
        &self,
    ) -> DeployedScriptInfo<{ RoyaltyPoolV1 as u8 }> {
        (&self.dex_deployment.royalty_pool).into()
    }
}

impl Has<PoolValidation> for RuntimeContext {
    fn select<U: IsEqual<PoolValidation>>(&self) -> PoolValidation {
        self.pool_validation.clone()
    }
}

impl Has<MinLovelacePerHarvest> for RuntimeContext {
    fn select<U: IsEqual<MinLovelacePerHarvest>>(&self) -> MinLovelacePerHarvest {
        self.harvest_limits.minimal_lovelace_per_single_harvest
    }
}

impl Has<DeployedScriptInfo<{ BufferWallet as u8 }>> for RuntimeContext {
    fn select<U: IsEqual<DeployedScriptInfo<{ BufferWallet as u8 }>>>(
        &self,
    ) -> DeployedScriptInfo<{ BufferWallet as u8 }> {
        (&self.dao_deployment.buffer_wallet.clone()).into()
    }
}

impl Has<NetworkId> for RuntimeContext {
    fn select<U: IsEqual<NetworkId>>(&self) -> NetworkId {
        self.network_id
    }
}

impl Has<GenesisEpochStartTime> for RuntimeContext {
    fn select<U: IsEqual<GenesisEpochStartTime>>(&self) -> GenesisEpochStartTime {
        self.genesis_epoch_start_time
    }
}

impl Has<OperatorCreds> for RuntimeContext {
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

has_deployed_script_info!(SmartFarm, RuntimeContext, |ctx: &RuntimeContext| (&ctx
    .dao_deployment
    .smart_farm)
    .into());
has_deployed_script_info!(HarvestOrder, RuntimeContext, |ctx: &RuntimeContext| (&ctx
    .dao_deployment
    .harvest_order)
    .into());
has_deployed_script_info!(PermManager, RuntimeContext, |ctx: &RuntimeContext| (&ctx
    .dao_deployment
    .perm_manager)
    .into());
has_deployed_validator!(PermManager, RuntimeContext, |ctx: &RuntimeContext| ctx
    .dao_deployment
    .perm_manager
    .clone());
has_deployed_script_info!(WpFactory, RuntimeContext, |ctx: &RuntimeContext| (&ctx
    .dao_deployment
    .wp_factory)
    .into());
has_deployed_script_info!(MintWpAuthPolicy, RuntimeContext, |ctx: &RuntimeContext| (&ctx
    .dao_deployment
    .mint_wpauth_token)
    .into());

impl Has<Collateral> for RuntimeContext {
    fn select<U: IsEqual<Collateral>>(&self) -> Collateral {
        todo!()
    }
}

impl Has<DeployedValidator<{ HarvestOrder as u8 }>> for RuntimeContext {
    fn select<U: IsEqual<DeployedValidator<{ HarvestOrder as u8 }>>>(
        &self,
    ) -> DeployedValidator<{ HarvestOrder as u8 }> {
        todo!()
    }
}
