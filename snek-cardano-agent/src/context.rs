use crate::snek_protocol_deployment::SnekProtocolDeployment;
use bloom_offchain::execution_engine::liquidity_book::config::ExecutionConfig;
use bloom_offchain::execution_engine::types::Time;
use bloom_offchain_cardano::orders::adhoc::AdhocFeeStructure;
use spectrum_cardano_lib::collateral::Collateral;
use spectrum_cardano_lib::ex_units::ExUnits;
use spectrum_cardano_lib::NetworkId;
use spectrum_offchain::backlog::BacklogCapacity;
use spectrum_offchain::data::Has;
use spectrum_offchain_cardano::creds::{OperatorCred, OperatorRewardAddress};
use spectrum_offchain_cardano::deployment::DeployedValidator;
use spectrum_offchain_cardano::deployment::ProtocolValidator::{
    DegenQuadraticPoolV1, LimitOrderV1, LimitOrderWitnessV1,
};
use type_equalities::IsEqual;

#[derive(Debug, Clone)]
pub struct MakerContext {
    pub time: Time,
    pub execution_conf: ExecutionConfig<ExUnits>,
    pub backlog_capacity: BacklogCapacity,
}

impl Has<BacklogCapacity> for MakerContext {
    fn select<U: IsEqual<BacklogCapacity>>(&self) -> BacklogCapacity {
        self.backlog_capacity
    }
}

impl Has<Time> for MakerContext {
    fn select<U: IsEqual<Time>>(&self) -> Time {
        self.time
    }
}

impl Has<ExecutionConfig<ExUnits>> for MakerContext {
    fn select<U: IsEqual<ExecutionConfig<ExUnits>>>(&self) -> ExecutionConfig<ExUnits> {
        self.execution_conf
    }
}

#[derive(Debug, Clone)]
pub struct ExecutionContext {
    pub time: Time,
    pub deployment: SnekProtocolDeployment,
    pub collateral: Collateral,
    pub reward_addr: OperatorRewardAddress,
    pub backlog_capacity: BacklogCapacity,
    pub network_id: NetworkId,
    pub operator_cred: OperatorCred,
    pub adhoc_fee_structure: AdhocFeeStructure,
}

impl Has<NetworkId> for ExecutionContext {
    fn select<U: IsEqual<NetworkId>>(&self) -> NetworkId {
        self.network_id
    }
}

impl Has<OperatorCred> for ExecutionContext {
    fn select<U: IsEqual<OperatorCred>>(&self) -> OperatorCred {
        self.operator_cred
    }
}

impl Has<BacklogCapacity> for ExecutionContext {
    fn select<U: IsEqual<BacklogCapacity>>(&self) -> BacklogCapacity {
        self.backlog_capacity
    }
}

impl Has<Time> for ExecutionContext {
    fn select<U: IsEqual<Time>>(&self) -> Time {
        self.time
    }
}

impl Has<Collateral> for ExecutionContext {
    fn select<U: IsEqual<Collateral>>(&self) -> Collateral {
        self.collateral.clone()
    }
}

impl Has<OperatorRewardAddress> for ExecutionContext {
    fn select<U: IsEqual<OperatorRewardAddress>>(&self) -> OperatorRewardAddress {
        self.reward_addr.clone()
    }
}

impl Has<AdhocFeeStructure> for ExecutionContext {
    fn select<U: IsEqual<AdhocFeeStructure>>(&self) -> AdhocFeeStructure {
        self.adhoc_fee_structure
    }
}

impl Has<DeployedValidator<{ LimitOrderV1 as u8 }>> for ExecutionContext {
    fn select<U: IsEqual<DeployedValidator<{ LimitOrderV1 as u8 }>>>(
        &self,
    ) -> DeployedValidator<{ LimitOrderV1 as u8 }> {
        self.deployment.limit_order.clone()
    }
}

impl Has<DeployedValidator<{ LimitOrderWitnessV1 as u8 }>> for ExecutionContext {
    fn select<U: IsEqual<DeployedValidator<{ LimitOrderWitnessV1 as u8 }>>>(
        &self,
    ) -> DeployedValidator<{ LimitOrderWitnessV1 as u8 }> {
        self.deployment.limit_order_witness.clone()
    }
}

impl Has<DeployedValidator<{ DegenQuadraticPoolV1 as u8 }>> for ExecutionContext {
    fn select<U: IsEqual<DeployedValidator<{ DegenQuadraticPoolV1 as u8 }>>>(
        &self,
    ) -> DeployedValidator<{ DegenQuadraticPoolV1 as u8 }> {
        self.deployment.degen_fn_pool_v1.clone()
    }
}
