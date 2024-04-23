use type_equalities::IsEqual;

use bloom_offchain::execution_engine::liquidity_book::ExecutionCap;
use bloom_offchain::execution_engine::types::Time;
use spectrum_cardano_lib::collateral::Collateral;
use spectrum_cardano_lib::ex_units::ExUnits;
use spectrum_cardano_lib::NetworkId;
use spectrum_offchain::backlog::BacklogCapacity;
use spectrum_offchain::data::Has;
use spectrum_offchain_cardano::creds::OperatorRewardAddress;
use spectrum_offchain_cardano::deployment::ProtocolValidator::{
    BalanceFnPoolDeposit, BalanceFnPoolRedeem, BalanceFnPoolV1, ConstFnPoolDeposit, ConstFnPoolFeeSwitch,
    ConstFnPoolFeeSwitchBiDirFee, ConstFnPoolRedeem, ConstFnPoolSwap, ConstFnPoolV1, ConstFnPoolV2,
    LimitOrderV1, LimitOrderWitnessV1,
};
use spectrum_offchain_cardano::deployment::{DeployedValidator, ProtocolDeployment};

#[derive(Debug, Clone)]
pub struct ExecutionContext {
    pub time: Time,
    pub execution_cap: ExecutionCap<ExUnits>,
    pub deployment: ProtocolDeployment,
    pub collateral: Collateral,
    pub reward_addr: OperatorRewardAddress,
    pub backlog_capacity: BacklogCapacity,
    pub network_id: NetworkId,
}

impl Has<NetworkId> for ExecutionContext {
    fn select<U: IsEqual<NetworkId>>(&self) -> NetworkId {
        self.network_id
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

impl Has<ExecutionCap<ExUnits>> for ExecutionContext {
    fn select<U: IsEqual<ExecutionCap<ExUnits>>>(&self) -> ExecutionCap<ExUnits> {
        self.execution_cap
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

impl Has<DeployedValidator<{ ConstFnPoolV1 as u8 }>> for ExecutionContext {
    fn select<U: IsEqual<DeployedValidator<{ ConstFnPoolV1 as u8 }>>>(
        &self,
    ) -> DeployedValidator<{ ConstFnPoolV1 as u8 }> {
        self.deployment.const_fn_pool_v1.clone()
    }
}

impl Has<DeployedValidator<{ ConstFnPoolV2 as u8 }>> for ExecutionContext {
    fn select<U: IsEqual<DeployedValidator<{ ConstFnPoolV2 as u8 }>>>(
        &self,
    ) -> DeployedValidator<{ ConstFnPoolV2 as u8 }> {
        self.deployment.const_fn_pool_v2.clone()
    }
}

impl Has<DeployedValidator<{ ConstFnPoolFeeSwitch as u8 }>> for ExecutionContext {
    fn select<U: IsEqual<DeployedValidator<{ ConstFnPoolFeeSwitch as u8 }>>>(
        &self,
    ) -> DeployedValidator<{ ConstFnPoolFeeSwitch as u8 }> {
        self.deployment.const_fn_pool_fee_switch.clone()
    }
}

impl Has<DeployedValidator<{ ConstFnPoolFeeSwitchBiDirFee as u8 }>> for ExecutionContext {
    fn select<U: IsEqual<DeployedValidator<{ ConstFnPoolFeeSwitchBiDirFee as u8 }>>>(
        &self,
    ) -> DeployedValidator<{ ConstFnPoolFeeSwitchBiDirFee as u8 }> {
        self.deployment.const_fn_pool_fee_switch_bidir_fee.clone()
    }
}

impl Has<DeployedValidator<{ ConstFnPoolSwap as u8 }>> for ExecutionContext {
    fn select<U: IsEqual<DeployedValidator<{ ConstFnPoolSwap as u8 }>>>(
        &self,
    ) -> DeployedValidator<{ ConstFnPoolSwap as u8 }> {
        self.deployment.const_fn_pool_swap.clone()
    }
}

impl Has<DeployedValidator<{ ConstFnPoolDeposit as u8 }>> for ExecutionContext {
    fn select<U: IsEqual<DeployedValidator<{ ConstFnPoolDeposit as u8 }>>>(
        &self,
    ) -> DeployedValidator<{ ConstFnPoolDeposit as u8 }> {
        self.deployment.const_fn_pool_deposit.clone()
    }
}

impl Has<DeployedValidator<{ ConstFnPoolRedeem as u8 }>> for ExecutionContext {
    fn select<U: IsEqual<DeployedValidator<{ ConstFnPoolRedeem as u8 }>>>(
        &self,
    ) -> DeployedValidator<{ ConstFnPoolRedeem as u8 }> {
        self.deployment.const_fn_pool_redeem.clone()
    }
}

impl Has<DeployedValidator<{ BalanceFnPoolV1 as u8 }>> for ExecutionContext {
    fn select<U: IsEqual<DeployedValidator<{ BalanceFnPoolV1 as u8 }>>>(
        &self,
    ) -> DeployedValidator<{ BalanceFnPoolV1 as u8 }> {
        self.deployment.balance_fn_pool_v1.clone()
    }
}

impl Has<DeployedValidator<{ BalanceFnPoolRedeem as u8 }>> for ExecutionContext {
    fn select<U: IsEqual<DeployedValidator<{ BalanceFnPoolRedeem as u8 }>>>(
        &self,
    ) -> DeployedValidator<{ BalanceFnPoolRedeem as u8 }> {
        self.deployment.balance_fn_pool_redeem.clone()
    }
}

impl Has<DeployedValidator<{ BalanceFnPoolDeposit as u8 }>> for ExecutionContext {
    fn select<U: IsEqual<DeployedValidator<{ BalanceFnPoolDeposit as u8 }>>>(
        &self,
    ) -> DeployedValidator<{ BalanceFnPoolDeposit as u8 }> {
        self.deployment.balance_fn_pool_deposit.clone()
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
