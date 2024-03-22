use type_equalities::IsEqual;

use bloom_offchain::execution_engine::liquidity_book::ExecutionCap;
use bloom_offchain::execution_engine::types::Time;
use bloom_offchain_cardano::creds::RewardAddress;
use spectrum_cardano_lib::collateral::Collateral;
use spectrum_offchain::backlog::BacklogCapacity;
use spectrum_offchain::data::Has;
use spectrum_offchain_cardano::deployment::ProtocolValidator::{
    ConstFnPoolV1, ConstFnPoolV2, LimitOrder, LimitOrderWitness,
};
use spectrum_offchain_cardano::deployment::{DeployedValidator, ProtocolDeployment};

#[derive(Debug, Clone)]
pub struct ExecutionContext {
    pub time: Time,
    pub execution_cap: ExecutionCap,
    pub deployment: ProtocolDeployment,
    pub collateral: Collateral,
    pub reward_addr: RewardAddress,
    pub backlog_capacity: BacklogCapacity,
    pub network_id: u8,
}

impl From<ExecutionContext> for spectrum_offchain_cardano::data::execution_context::ExecutionContext {
    fn from(value: ExecutionContext) -> Self {
        Self {
            operator_addr: value.reward_addr.into(),
            ref_scripts: todo!("Legacy EC should be using ProtocolDeployment as well"),
            collateral: value.collateral,
            network_id: value.network_id,
        }
    }
}

impl Has<BacklogCapacity> for ExecutionContext {
    fn get_labeled<U: IsEqual<BacklogCapacity>>(&self) -> BacklogCapacity {
        self.backlog_capacity
    }
}

impl Has<Time> for ExecutionContext {
    fn get_labeled<U: IsEqual<Time>>(&self) -> Time {
        self.time
    }
}

impl Has<ExecutionCap> for ExecutionContext {
    fn get_labeled<U: IsEqual<ExecutionCap>>(&self) -> ExecutionCap {
        self.execution_cap
    }
}

impl Has<Collateral> for ExecutionContext {
    fn get_labeled<U: IsEqual<Collateral>>(&self) -> Collateral {
        self.collateral.clone()
    }
}

impl Has<RewardAddress> for ExecutionContext {
    fn get_labeled<U: IsEqual<RewardAddress>>(&self) -> RewardAddress {
        self.reward_addr.clone()
    }
}

impl Has<DeployedValidator<{ ConstFnPoolV1 as u8 }>> for ExecutionContext {
    fn get_labeled<U: IsEqual<DeployedValidator<{ ConstFnPoolV1 as u8 }>>>(
        &self,
    ) -> DeployedValidator<{ ConstFnPoolV1 as u8 }> {
        self.deployment.const_fn_pool_v1.clone()
    }
}

impl Has<DeployedValidator<{ ConstFnPoolV2 as u8 }>> for ExecutionContext {
    fn get_labeled<U: IsEqual<DeployedValidator<{ ConstFnPoolV2 as u8 }>>>(
        &self,
    ) -> DeployedValidator<{ ConstFnPoolV2 as u8 }> {
        self.deployment.const_fn_pool_v2.clone()
    }
}

impl Has<DeployedValidator<{ LimitOrder as u8 }>> for ExecutionContext {
    fn get_labeled<U: IsEqual<DeployedValidator<{ LimitOrder as u8 }>>>(
        &self,
    ) -> DeployedValidator<{ LimitOrder as u8 }> {
        self.deployment.limit_order.clone()
    }
}

impl Has<DeployedValidator<{ LimitOrderWitness as u8 }>> for ExecutionContext {
    fn get_labeled<U: IsEqual<DeployedValidator<{ LimitOrderWitness as u8 }>>>(
        &self,
    ) -> DeployedValidator<{ LimitOrderWitness as u8 }> {
        self.deployment.limit_order_witness.clone()
    }
}
