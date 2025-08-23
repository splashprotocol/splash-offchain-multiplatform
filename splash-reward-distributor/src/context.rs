use crate::config::HarvestLimits;
use spectrum_cardano_lib::collateral::Collateral;
use spectrum_cardano_lib::NetworkId;
use spectrum_offchain::domain::Has;
use spectrum_offchain_cardano::deployment::DeployedValidator;
use spectrum_offchain_cardano::{has_deployed_script_info, has_deployed_validator};
use splash_dao_offchain::deployment::ProtocolValidator::*;
use splash_dao_offchain::protocol_config::BufferWalletScript;
use splash_dao_offchain::protocol_config::{OperatorCreds, PermManagerAuthPolicy, SplashPolicy};
use splash_yf_offchain::settings::MinLovelacePerHarvest;
use type_equalities::IsEqual;

#[derive(Debug, Clone)]
pub struct RuntimeContext {}

has_deployed_validator!(SmartFarm, RuntimeContext, |ctx: &RuntimeContext| todo!());
has_deployed_script_info!(SmartFarm, RuntimeContext, |ctx: &RuntimeContext| todo!());
has_deployed_script_info!(HarvestOrder, RuntimeContext, |ctx: &RuntimeContext| todo!());
has_deployed_script_info!(PermManager, RuntimeContext, |ctx: &RuntimeContext| todo!());

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

impl Has<DeployedValidator<{ PermManager as u8 }>> for RuntimeContext {
    fn select<U: IsEqual<DeployedValidator<{ PermManager as u8 }>>>(
        &self,
    ) -> DeployedValidator<{ PermManager as u8 }> {
        todo!()
    }
}

impl Has<BufferWalletScript> for RuntimeContext {
    fn select<U: IsEqual<BufferWalletScript>>(&self) -> BufferWalletScript {
        todo!()
    }
}

impl Has<MinLovelacePerHarvest> for RuntimeContext {
    fn select<U: IsEqual<MinLovelacePerHarvest>>(&self) -> MinLovelacePerHarvest {
        todo!()
    }
}

impl Has<NetworkId> for RuntimeContext {
    fn select<U: IsEqual<NetworkId>>(&self) -> NetworkId {
        todo!()
    }
}

impl Has<OperatorCreds> for RuntimeContext {
    fn select<U: IsEqual<OperatorCreds>>(&self) -> OperatorCreds {
        todo!()
    }
}

impl Has<SplashPolicy> for RuntimeContext {
    fn select<U: IsEqual<SplashPolicy>>(&self) -> SplashPolicy {
        todo!()
    }
}

impl Has<PermManagerAuthPolicy> for RuntimeContext {
    fn select<U: IsEqual<PermManagerAuthPolicy>>(&self) -> PermManagerAuthPolicy {
        todo!()
    }
}
