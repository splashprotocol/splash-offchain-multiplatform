use crate::config::HarvestLimits;
use spectrum_cardano_lib::collateral::Collateral;
use spectrum_cardano_lib::NetworkId;
use spectrum_offchain::domain::Has;
use spectrum_offchain_cardano::has_deployed_script_info;
use splash_dao_offchain::deployment::ProtocolValidator::*;
use splash_dao_offchain::protocol_config::{
    BufferWalletScript, FarmAuthRefScriptOutput, HarvestOrderRefScriptOutput, HarvestOrderScriptHash,
    PermManagerBoxRefScriptOutput,
};
use splash_dao_offchain::protocol_config::{
    FarmAuthPolicy, OperatorCreds, PermManagerAuthPolicy, SplashPolicy,
};
use type_equalities::IsEqual;

#[derive(Debug, Clone)]
pub struct RuntimeContext {}

has_deployed_script_info!(SmartFarm, RuntimeContext, |ctx: &RuntimeContext| todo!());
has_deployed_script_info!(HarvestOrder, RuntimeContext, |ctx: &RuntimeContext| todo!());
has_deployed_script_info!(PermManager, RuntimeContext, |ctx: &RuntimeContext| todo!());

impl Has<Collateral> for RuntimeContext {
    fn select<U: IsEqual<Collateral>>(&self) -> Collateral {
        todo!()
    }
}

impl Has<HarvestOrderRefScriptOutput> for RuntimeContext {
    fn select<U: IsEqual<HarvestOrderRefScriptOutput>>(&self) -> HarvestOrderRefScriptOutput {
        todo!()
    }
}

impl Has<HarvestOrderScriptHash> for RuntimeContext {
    fn select<U: IsEqual<HarvestOrderScriptHash>>(&self) -> HarvestOrderScriptHash {
        todo!()
    }
}

impl Has<PermManagerBoxRefScriptOutput> for RuntimeContext {
    fn select<U: IsEqual<PermManagerBoxRefScriptOutput>>(&self) -> PermManagerBoxRefScriptOutput {
        todo!()
    }
}

impl Has<FarmAuthRefScriptOutput> for RuntimeContext {
    fn select<U: IsEqual<FarmAuthRefScriptOutput>>(&self) -> FarmAuthRefScriptOutput {
        todo!()
    }
}

impl Has<BufferWalletScript> for RuntimeContext {
    fn select<U: IsEqual<BufferWalletScript>>(&self) -> BufferWalletScript {
        todo!()
    }
}

impl Has<HarvestLimits> for RuntimeContext {
    fn select<U: IsEqual<HarvestLimits>>(&self) -> HarvestLimits {
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

impl Has<FarmAuthPolicy> for RuntimeContext {
    fn select<U: IsEqual<FarmAuthPolicy>>(&self) -> FarmAuthPolicy {
        todo!()
    }
}
