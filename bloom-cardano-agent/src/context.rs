use bloom_offchain::execution_engine::liquidity_book::config::ExecutionConfig;
use bloom_offchain::execution_engine::types::Time;
use spectrum_cardano_lib::collateral::Collateral;
use spectrum_cardano_lib::ex_units::ExUnits;
use spectrum_cardano_lib::NetworkId;
use spectrum_offchain::backlog::BacklogCapacity;
use spectrum_offchain::domain::Has;
use spectrum_offchain_cardano::creds::{OperatorCred, OperatorRewardAddress};
use spectrum_offchain_cardano::deployment::ProtocolValidator::{
    BalanceFnPoolDeposit, BalanceFnPoolRedeem, BalanceFnPoolV1, BalanceFnPoolV2, ConstFnFeeSwitchPoolDeposit,
    ConstFnFeeSwitchPoolRedeem, ConstFnFeeSwitchPoolSwap, ConstFnPoolDeposit, ConstFnPoolFeeSwitch,
    ConstFnPoolFeeSwitchBiDirFee, ConstFnPoolFeeSwitchV2, ConstFnPoolRedeem, ConstFnPoolSwap, ConstFnPoolV1,
    ConstFnPoolV2, GridOrderNative, LimitOrderV1, LimitOrderWitnessV1, StableFnPoolT2T,
    StableFnPoolT2TDeposit, StableFnPoolT2TRedeem,
};
use spectrum_offchain_cardano::deployment::{DeployedValidator, ProtocolDeployment};
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
    pub deployment: ProtocolDeployment,
    pub collateral: Collateral,
    pub reward_addr: OperatorRewardAddress,
    pub backlog_capacity: BacklogCapacity,
    pub network_id: NetworkId,
    pub operator_cred: OperatorCred,
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

impl Has<DeployedValidator<{ ConstFnPoolFeeSwitchV2 as u8 }>> for ExecutionContext {
    fn select<U: IsEqual<DeployedValidator<{ ConstFnPoolFeeSwitchV2 as u8 }>>>(
        &self,
    ) -> DeployedValidator<{ ConstFnPoolFeeSwitchV2 as u8 }> {
        self.deployment.const_fn_pool_fee_switch_v2.clone()
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

impl Has<DeployedValidator<{ ConstFnFeeSwitchPoolSwap as u8 }>> for ExecutionContext {
    fn select<U: IsEqual<DeployedValidator<{ ConstFnFeeSwitchPoolSwap as u8 }>>>(
        &self,
    ) -> DeployedValidator<{ ConstFnFeeSwitchPoolSwap as u8 }> {
        self.deployment.const_fn_fee_switch_pool_swap.clone()
    }
}

impl Has<DeployedValidator<{ ConstFnFeeSwitchPoolDeposit as u8 }>> for ExecutionContext {
    fn select<U: IsEqual<DeployedValidator<{ ConstFnFeeSwitchPoolDeposit as u8 }>>>(
        &self,
    ) -> DeployedValidator<{ ConstFnFeeSwitchPoolDeposit as u8 }> {
        self.deployment.const_fn_fee_switch_pool_deposit.clone()
    }
}

impl Has<DeployedValidator<{ ConstFnFeeSwitchPoolRedeem as u8 }>> for ExecutionContext {
    fn select<U: IsEqual<DeployedValidator<{ ConstFnFeeSwitchPoolRedeem as u8 }>>>(
        &self,
    ) -> DeployedValidator<{ ConstFnFeeSwitchPoolRedeem as u8 }> {
        self.deployment.const_fn_fee_switch_pool_redeem.clone()
    }
}

impl Has<DeployedValidator<{ BalanceFnPoolV1 as u8 }>> for ExecutionContext {
    fn select<U: IsEqual<DeployedValidator<{ BalanceFnPoolV1 as u8 }>>>(
        &self,
    ) -> DeployedValidator<{ BalanceFnPoolV1 as u8 }> {
        self.deployment.balance_fn_pool_v1.clone()
    }
}

impl Has<DeployedValidator<{ BalanceFnPoolV2 as u8 }>> for ExecutionContext {
    fn select<U: IsEqual<DeployedValidator<{ BalanceFnPoolV2 as u8 }>>>(
        &self,
    ) -> DeployedValidator<{ BalanceFnPoolV2 as u8 }> {
        self.deployment.balance_fn_pool_v2.clone()
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

impl Has<DeployedValidator<{ StableFnPoolT2T as u8 }>> for ExecutionContext {
    fn select<U: IsEqual<DeployedValidator<{ StableFnPoolT2T as u8 }>>>(
        &self,
    ) -> DeployedValidator<{ StableFnPoolT2T as u8 }> {
        self.deployment.stable_fn_pool_t2t.clone()
    }
}

impl Has<DeployedValidator<{ StableFnPoolT2TDeposit as u8 }>> for ExecutionContext {
    fn select<U: IsEqual<DeployedValidator<{ StableFnPoolT2TDeposit as u8 }>>>(
        &self,
    ) -> DeployedValidator<{ StableFnPoolT2TDeposit as u8 }> {
        self.deployment.stable_fn_pool_t2t_deposit.clone()
    }
}

impl Has<DeployedValidator<{ StableFnPoolT2TRedeem as u8 }>> for ExecutionContext {
    fn select<U: IsEqual<DeployedValidator<{ StableFnPoolT2TRedeem as u8 }>>>(
        &self,
    ) -> DeployedValidator<{ StableFnPoolT2TRedeem as u8 }> {
        self.deployment.stable_fn_pool_t2t_redeem.clone()
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

impl Has<DeployedValidator<{ GridOrderNative as u8 }>> for ExecutionContext {
    fn select<U: IsEqual<DeployedValidator<{ GridOrderNative as u8 }>>>(
        &self,
    ) -> DeployedValidator<{ GridOrderNative as u8 }> {
        self.deployment.grid_order_native.clone()
    }
}
