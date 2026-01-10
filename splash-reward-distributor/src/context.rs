use crate::engine::verifier::AuthorizedExecutors;
use cml_chain::address::{Address, EnterpriseAddress};
use cml_chain::certs::Credential;
use cml_crypto::{Ed25519KeyHash, PrivateKey, ScriptHash};
use spectrum_cardano_lib::collateral::Collateral;
use spectrum_cardano_lib::NetworkId;
use spectrum_offchain::domain::Has;
use spectrum_offchain_cardano::creds::operator_creds;
use spectrum_offchain_cardano::{has_deployed_script_info, has_deployed_validator};
use splash_dao_offchain::deployment::{
    ProtocolDeployment as DaoDeployment, ProtocolTokens as DaoTokens, ProtocolValidator::*,
};
use splash_dao_offchain::protocol_config::{
    BufferWalletAuthPolicy, FarmFactoryAuthPolicy, OperatorCreds, PermManagerAuthPolicy, SplashPolicy,
};
use splash_dao_offchain::GenesisEpochStartTime;
use splash_yf_offchain::settings::MinLovelacePerHarvest;
use std::ops::Index;
use type_equalities::IsEqual;

#[derive(Clone)]
pub struct VerifierRuntimeContext {
    pub dao_deployment: DaoDeployment,
    pub dao_tokens: DaoTokens,
    pub min_lovelace_per_harvest: MinLovelacePerHarvest,
    pub splash_policy_id: ScriptHash,
    pub network_id: NetworkId,
    pub genesis_epoch_start_time: GenesisEpochStartTime,
    pub authorized_executors: AuthorizedExecutors,
    pub reward_tx_ttl: RewardTxTtl,
    pub operator_sk: String,
}

has_deployed_validator!(
    SmartFarm,
    VerifierRuntimeContext,
    |ctx: &VerifierRuntimeContext| ctx.dao_deployment.smart_farm.clone()
);
has_deployed_script_info!(
    SmartFarm,
    VerifierRuntimeContext,
    |ctx: &VerifierRuntimeContext| (&ctx.dao_deployment.smart_farm).into()
);
has_deployed_validator!(
    FarmFactory,
    VerifierRuntimeContext,
    |ctx: &VerifierRuntimeContext| ctx.dao_deployment.farm_factory.clone()
);
has_deployed_script_info!(
    FarmFactory,
    VerifierRuntimeContext,
    |ctx: &VerifierRuntimeContext| (&ctx.dao_deployment.farm_factory).into()
);
has_deployed_validator!(
    MintWpAuthPolicy,
    VerifierRuntimeContext,
    |ctx: &VerifierRuntimeContext| ctx.dao_deployment.mint_wpauth_token.clone()
);
has_deployed_script_info!(
    MintWpAuthPolicy,
    VerifierRuntimeContext,
    |ctx: &VerifierRuntimeContext| (&ctx.dao_deployment.mint_wpauth_token).into()
);
has_deployed_validator!(
    HarvestOrder,
    VerifierRuntimeContext,
    |ctx: &VerifierRuntimeContext| ctx.dao_deployment.harvest_order.clone()
);
has_deployed_script_info!(
    HarvestOrder,
    VerifierRuntimeContext,
    |ctx: &VerifierRuntimeContext| (&ctx.dao_deployment.harvest_order).into()
);
has_deployed_validator!(
    PermManager,
    VerifierRuntimeContext,
    |ctx: &VerifierRuntimeContext| ctx.dao_deployment.perm_manager.clone()
);
has_deployed_script_info!(
    PermManager,
    VerifierRuntimeContext,
    |ctx: &VerifierRuntimeContext| (&ctx.dao_deployment.perm_manager).into()
);

has_deployed_validator!(
    BufferWallet,
    VerifierRuntimeContext,
    |ctx: &VerifierRuntimeContext| ctx.dao_deployment.buffer_wallet.clone()
);
has_deployed_script_info!(
    BufferWallet,
    VerifierRuntimeContext,
    |ctx: &VerifierRuntimeContext| (&ctx.dao_deployment.buffer_wallet).into()
);

impl Has<BufferWalletAuthPolicy> for VerifierRuntimeContext {
    fn select<U: IsEqual<BufferWalletAuthPolicy>>(&self) -> BufferWalletAuthPolicy {
        BufferWalletAuthPolicy(self.dao_tokens.buffer_wallet.policy_id)
    }
}

impl Has<MinLovelacePerHarvest> for VerifierRuntimeContext {
    fn select<U: IsEqual<MinLovelacePerHarvest>>(&self) -> MinLovelacePerHarvest {
        self.min_lovelace_per_harvest
    }
}

impl Has<NetworkId> for VerifierRuntimeContext {
    fn select<U: IsEqual<NetworkId>>(&self) -> NetworkId {
        self.network_id
    }
}

impl Has<SplashPolicy> for VerifierRuntimeContext {
    fn select<U: IsEqual<SplashPolicy>>(&self) -> SplashPolicy {
        SplashPolicy(self.splash_policy_id)
    }
}

impl Has<PermManagerAuthPolicy> for VerifierRuntimeContext {
    fn select<U: IsEqual<PermManagerAuthPolicy>>(&self) -> PermManagerAuthPolicy {
        PermManagerAuthPolicy(self.dao_tokens.perm_auth.policy_id)
    }
}

impl Has<FarmFactoryAuthPolicy> for VerifierRuntimeContext {
    fn select<U: IsEqual<FarmFactoryAuthPolicy>>(&self) -> FarmFactoryAuthPolicy {
        FarmFactoryAuthPolicy(self.dao_tokens.factory_auth.policy_id)
    }
}

impl Has<GenesisEpochStartTime> for VerifierRuntimeContext {
    fn select<U: IsEqual<GenesisEpochStartTime>>(&self) -> GenesisEpochStartTime {
        self.genesis_epoch_start_time
    }
}

impl Has<AuthorizedExecutors> for VerifierRuntimeContext {
    fn select<U: IsEqual<AuthorizedExecutors>>(&self) -> AuthorizedExecutors {
        self.authorized_executors.clone()
    }
}

/// Provide dummy operator credentials here to satisfy trait bounds in `event_pipeline()`
impl Has<OperatorCreds> for VerifierRuntimeContext {
    fn select<U: IsEqual<OperatorCreds>>(&self) -> OperatorCreds {
        let (operator_cred, _, funding_addresses) = operator_creds(&self.operator_sk, self.network_id);
        OperatorCreds(operator_cred.0, funding_addresses.index(0).clone())
    }
}

impl Has<PrivateKey> for VerifierRuntimeContext {
    fn select<U: IsEqual<PrivateKey>>(&self) -> PrivateKey {
        let bip32_key = cml_crypto::Bip32PrivateKey::from_bech32(self.operator_sk.as_str()).unwrap();
        bip32_key.to_raw_key()
    }
}

/// TX TTL for both harvest and gauge buffering TXs (specified in # slots).
#[derive(Clone, Copy)]
pub struct RewardTxTtl(pub u64);

impl Has<RewardTxTtl> for VerifierRuntimeContext {
    fn select<U: IsEqual<RewardTxTtl>>(&self) -> RewardTxTtl {
        self.reward_tx_ttl
    }
}

#[derive(Clone)]
pub struct RewardBotRuntimeContext {
    pub verifier_runtime_context: VerifierRuntimeContext,
    pub collateral: Collateral,
}

impl Has<Collateral> for RewardBotRuntimeContext {
    fn select<U: IsEqual<Collateral>>(&self) -> Collateral {
        self.collateral.clone()
    }
}

impl Has<OperatorCreds> for RewardBotRuntimeContext {
    fn select<U: IsEqual<OperatorCreds>>(&self) -> OperatorCreds {
        let (operator_cred, _, funding_addresses) = operator_creds(
            &self.verifier_runtime_context.operator_sk,
            self.verifier_runtime_context.network_id,
        );
        OperatorCreds(operator_cred.0, funding_addresses.index(0).clone())
    }
}

impl Has<PrivateKey> for RewardBotRuntimeContext {
    fn select<U: IsEqual<PrivateKey>>(&self) -> PrivateKey {
        let bip32_key =
            cml_crypto::Bip32PrivateKey::from_bech32(self.verifier_runtime_context.operator_sk.as_str())
                .unwrap();
        bip32_key.to_raw_key()
    }
}

has_deployed_validator!(
    SmartFarm,
    RewardBotRuntimeContext,
    |ctx: &RewardBotRuntimeContext| ctx.verifier_runtime_context.dao_deployment.smart_farm.clone()
);
has_deployed_script_info!(
    SmartFarm,
    RewardBotRuntimeContext,
    |ctx: &RewardBotRuntimeContext| (&ctx.verifier_runtime_context.dao_deployment.smart_farm).into()
);
has_deployed_validator!(
    FarmFactory,
    RewardBotRuntimeContext,
    |ctx: &RewardBotRuntimeContext| ctx.verifier_runtime_context.dao_deployment.farm_factory.clone()
);
has_deployed_script_info!(
    FarmFactory,
    RewardBotRuntimeContext,
    |ctx: &RewardBotRuntimeContext| (&ctx.verifier_runtime_context.dao_deployment.farm_factory).into()
);
has_deployed_validator!(
    MintWpAuthPolicy,
    RewardBotRuntimeContext,
    |ctx: &RewardBotRuntimeContext| ctx
        .verifier_runtime_context
        .dao_deployment
        .mint_wpauth_token
        .clone()
);
has_deployed_script_info!(
    MintWpAuthPolicy,
    RewardBotRuntimeContext,
    |ctx: &RewardBotRuntimeContext| (&ctx.verifier_runtime_context.dao_deployment.mint_wpauth_token).into()
);
has_deployed_validator!(
    HarvestOrder,
    RewardBotRuntimeContext,
    |ctx: &RewardBotRuntimeContext| ctx.verifier_runtime_context.dao_deployment.harvest_order.clone()
);
has_deployed_script_info!(
    HarvestOrder,
    RewardBotRuntimeContext,
    |ctx: &RewardBotRuntimeContext| (&ctx.verifier_runtime_context.dao_deployment.harvest_order).into()
);
has_deployed_validator!(
    PermManager,
    RewardBotRuntimeContext,
    |ctx: &RewardBotRuntimeContext| ctx.verifier_runtime_context.dao_deployment.perm_manager.clone()
);
has_deployed_script_info!(
    PermManager,
    RewardBotRuntimeContext,
    |ctx: &RewardBotRuntimeContext| (&ctx.verifier_runtime_context.dao_deployment.perm_manager).into()
);
has_deployed_validator!(
    BufferWallet,
    RewardBotRuntimeContext,
    |ctx: &RewardBotRuntimeContext| ctx.verifier_runtime_context.dao_deployment.buffer_wallet.clone()
);
has_deployed_script_info!(
    BufferWallet,
    RewardBotRuntimeContext,
    |ctx: &RewardBotRuntimeContext| (&ctx.verifier_runtime_context.dao_deployment.buffer_wallet).into()
);

impl Has<BufferWalletAuthPolicy> for RewardBotRuntimeContext {
    fn select<U: IsEqual<BufferWalletAuthPolicy>>(&self) -> BufferWalletAuthPolicy {
        BufferWalletAuthPolicy(self.verifier_runtime_context.dao_tokens.buffer_wallet.policy_id)
    }
}

impl Has<MinLovelacePerHarvest> for RewardBotRuntimeContext {
    fn select<U: IsEqual<MinLovelacePerHarvest>>(&self) -> MinLovelacePerHarvest {
        self.verifier_runtime_context.min_lovelace_per_harvest
    }
}

impl Has<NetworkId> for RewardBotRuntimeContext {
    fn select<U: IsEqual<NetworkId>>(&self) -> NetworkId {
        self.verifier_runtime_context.network_id
    }
}

impl Has<SplashPolicy> for RewardBotRuntimeContext {
    fn select<U: IsEqual<SplashPolicy>>(&self) -> SplashPolicy {
        SplashPolicy(self.verifier_runtime_context.splash_policy_id)
    }
}

impl Has<PermManagerAuthPolicy> for RewardBotRuntimeContext {
    fn select<U: IsEqual<PermManagerAuthPolicy>>(&self) -> PermManagerAuthPolicy {
        PermManagerAuthPolicy(self.verifier_runtime_context.dao_tokens.perm_auth.policy_id)
    }
}

impl Has<FarmFactoryAuthPolicy> for RewardBotRuntimeContext {
    fn select<U: IsEqual<FarmFactoryAuthPolicy>>(&self) -> FarmFactoryAuthPolicy {
        FarmFactoryAuthPolicy(self.verifier_runtime_context.dao_tokens.factory_auth.policy_id)
    }
}

impl Has<GenesisEpochStartTime> for RewardBotRuntimeContext {
    fn select<U: IsEqual<GenesisEpochStartTime>>(&self) -> GenesisEpochStartTime {
        self.verifier_runtime_context.genesis_epoch_start_time
    }
}

impl Has<AuthorizedExecutors> for RewardBotRuntimeContext {
    fn select<U: IsEqual<AuthorizedExecutors>>(&self) -> AuthorizedExecutors {
        self.verifier_runtime_context.authorized_executors.clone()
    }
}

impl Has<RewardTxTtl> for RewardBotRuntimeContext {
    fn select<U: IsEqual<RewardTxTtl>>(&self) -> RewardTxTtl {
        self.verifier_runtime_context.reward_tx_ttl
    }
}
