use crate::config::withdraw_config::WithdrawConfig;
use crate::onchain::event::{BufferWalletAddress, BufferedWallet};
use actix_web::App;
use cardano_explorer::CardanoNetwork;
use cml_chain::PolicyId;
use cml_crypto::ScriptHash;
use spectrum_cardano_lib::NetworkId;
use spectrum_offchain::domain::Has;
use spectrum_offchain_cardano::creds::operator_creds;
use spectrum_offchain_cardano::deployment::{DeployedScriptInfo, DeployedValidator};
use splash_dao_offchain::deployment::ProtocolValidator::SmartFarm;
use splash_dao_offchain::deployment::{IssuedAsset, ProtocolValidator};
use splash_dao_offchain::protocol_config::{
    FarmAuthPolicy, PermManagerAuthPolicy, ProtocolConfig, SplashPolicy,
};
use splash_distribution::context::DistributorCreds;
use splash_lp_indexer::context::Context;
use type_equalities::IsEqual;
use splash_lp_indexer::config::HarvestLimits;

#[derive(Clone)]
pub struct AppContext {
    harvest_order: DeployedValidator<{ ProtocolValidator::HarvestOrder as u8 }>,
    smart_farm: DeployedValidator<{ ProtocolValidator::SmartFarm as u8 }>,
    perm_manager: DeployedValidator<{ ProtocolValidator::PermManager as u8 }>,
    distributor_creds: DistributorCreds,
    harvest_limits: HarvestLimits,
    network_id: NetworkId,
    perm_auth: IssuedAsset,
    splash_policy_id: PolicyId,
    buffered_wallet: BufferWalletAddress,
}

impl AppContext {
    pub async fn from_config<Net: CardanoNetwork>(
        withdraw_config: WithdrawConfig,
        explorer: &Net,
    ) -> AppContext {
        let harvest_order =
            DeployedValidator::unsafe_pull(withdraw_config.clone().harvest_order, explorer).await;

        let smart_farm = DeployedValidator::unsafe_pull(withdraw_config.clone().smart_farm, explorer).await;

        let perm_manager = DeployedValidator::unsafe_pull(withdraw_config.clone().perm_manager, explorer).await;

        let (distributor_cred, distributor_address, _) =
            operator_creds(&withdraw_config.distributor_key, withdraw_config.network_id);

        let buffer_wallet_address = ScriptHash::from_hex(withdraw_config.buffered_wallet.as_str()).unwrap();

        AppContext {
            harvest_order,
            smart_farm,
            distributor_creds: DistributorCreds(distributor_cred.0, distributor_address.0),
            harvest_limits: withdraw_config.harvest_limits,
            network_id: withdraw_config.network_id,
            perm_auth: withdraw_config.perm_auth,
            perm_manager: perm_manager,
            splash_policy_id: withdraw_config.splash_policy_id,
            buffered_wallet: BufferWalletAddress(buffer_wallet_address),
        }
    }
}

impl Has<DeployedValidator<{ ProtocolValidator::HarvestOrder as u8 }>> for AppContext {
    fn select<U: IsEqual<DeployedValidator<{ ProtocolValidator::HarvestOrder as u8 }>>>(
        &self,
    ) -> DeployedValidator<{ ProtocolValidator::HarvestOrder as u8 }> {
        self.harvest_order.clone()
    }
}

impl Has<DeployedValidator<{ ProtocolValidator::SmartFarm as u8 }>> for AppContext {
    fn select<U: IsEqual<DeployedValidator<{ ProtocolValidator::SmartFarm as u8 }>>>(
        &self,
    ) -> DeployedValidator<{ ProtocolValidator::SmartFarm as u8 }> {
        self.smart_farm.clone()
    }
}

impl Has<DeployedScriptInfo<{ ProtocolValidator::PermManager as u8 }>> for AppContext {
    fn select<U: IsEqual<DeployedScriptInfo<{ ProtocolValidator::PermManager as u8 }>>>(
        &self,
    ) -> DeployedScriptInfo<{ ProtocolValidator::PermManager as u8 }> {
        From::from(&self.perm_manager)
    }
}

impl Has<DeployedScriptInfo<{ ProtocolValidator::HarvestOrder as u8 }>> for AppContext {
    fn select<U: IsEqual<DeployedScriptInfo<{ ProtocolValidator::HarvestOrder as u8 }>>>(
        &self,
    ) -> DeployedScriptInfo<{ ProtocolValidator::HarvestOrder as u8 }> {
        From::from(&self.harvest_order)
    }
}

impl Has<DistributorCreds> for AppContext {
    fn select<U: IsEqual<DistributorCreds>>(&self) -> DistributorCreds {
        self.distributor_creds.clone()
    }
}

impl Has<HarvestLimits> for AppContext {
    fn select<U: IsEqual<HarvestLimits>>(&self) -> HarvestLimits {
        self.harvest_limits.clone()
    }
}

impl Has<FarmAuthPolicy> for AppContext {
    fn select<U: IsEqual<FarmAuthPolicy>>(&self) -> FarmAuthPolicy {
        FarmAuthPolicy(self.smart_farm.clone().hash)
    }
}

impl Has<NetworkId> for AppContext {
    fn select<U: IsEqual<NetworkId>>(&self) -> NetworkId {
        self.network_id.clone()
    }
}

impl Has<PermManagerAuthPolicy> for AppContext {
    fn select<U: IsEqual<PermManagerAuthPolicy>>(&self) -> PermManagerAuthPolicy {
        PermManagerAuthPolicy(self.perm_auth.policy_id)
    }
}

impl Has<SplashPolicy> for AppContext {
    fn select<U: IsEqual<SplashPolicy>>(&self) -> SplashPolicy {
        SplashPolicy(self.splash_policy_id)
    }
}

impl Has<DeployedScriptInfo<{ SmartFarm as u8 }>> for AppContext {
    fn select<U: IsEqual<DeployedScriptInfo<{ SmartFarm as u8 }>>>(
        &self,
    ) -> DeployedScriptInfo<{ SmartFarm as u8 }> {
        (&self.smart_farm).into()
    }
}

impl Has<BufferWalletAddress> for AppContext {
    fn select<U: IsEqual<BufferWalletAddress>>(&self) -> BufferWalletAddress {
        self.buffered_wallet.clone()
    }
}