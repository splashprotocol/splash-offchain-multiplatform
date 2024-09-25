use bloom_offchain_cardano::event_sink::context::EventContext;
use bloom_offchain_cardano::orders::adhoc::AdhocFeeStructure;
use bloom_offchain_cardano::orders::limit::LimitOrderValidation;
use bloom_offchain_cardano::validation_rules::ValidationRules;
use cml_chain::auxdata::Metadata;
use type_equalities::IsEqual;

use spectrum_cardano_lib::OutputRef;
use spectrum_offchain::data::Has;
use spectrum_offchain_cardano::creds::OperatorCred;
use spectrum_offchain_cardano::data::deposit::DepositOrderValidation;
use spectrum_offchain_cardano::data::pool::PoolValidation;
use spectrum_offchain_cardano::data::redeem::RedeemOrderValidation;
use spectrum_offchain_cardano::deployment::ProtocolValidator::{
    BalanceFnPoolDeposit, BalanceFnPoolRedeem, BalanceFnPoolV1, BalanceFnPoolV2, ConstFnFeeSwitchPoolDeposit,
    ConstFnFeeSwitchPoolRedeem, ConstFnFeeSwitchPoolSwap, ConstFnPoolDeposit, ConstFnPoolFeeSwitch,
    ConstFnPoolFeeSwitchBiDirFee, ConstFnPoolFeeSwitchV2, ConstFnPoolRedeem, ConstFnPoolSwap, ConstFnPoolV1,
    ConstFnPoolV2, DegenQuadraticPoolV1, LimitOrderV1, LimitOrderWitnessV1, StableFnPoolT2T,
    StableFnPoolT2TDeposit, StableFnPoolT2TRedeem,
};
use spectrum_offchain_cardano::deployment::{DeployedScriptInfo, ProtocolScriptHashes};
use spectrum_offchain_cardano::handler_context::{
    AuthVerificationKey, ConsumedIdentifiers, ConsumedInputs, ProducedIdentifiers,
};

#[derive(Copy, Clone, Debug)]
pub struct SnekHandlerContextProto {
    pub executor_cred: OperatorCred,
    pub scripts: ProtocolScriptHashes,
    pub validation_rules: ValidationRules,
    pub adhoc_fee_structure: AdhocFeeStructure,
    pub auth_verification_key: AuthVerificationKey,
}

#[derive(Clone, Debug)]
pub struct SnekHandlerContext<I: Copy> {
    pub output_ref: OutputRef,
    pub metadata: Option<Metadata>,
    pub consumed_utxos: ConsumedInputs,
    pub consumed_identifiers: ConsumedIdentifiers<I>,
    pub produced_identifiers: ProducedIdentifiers<I>,
    pub executor_cred: OperatorCred,
    pub scripts: ProtocolScriptHashes,
    pub bounds: ValidationRules,
    pub adhoc_fee_structure: AdhocFeeStructure,
    pub auth_verification_key: AuthVerificationKey,
}

impl<I: Copy> From<(SnekHandlerContextProto, EventContext<I>)> for SnekHandlerContext<I> {
    fn from(value: (SnekHandlerContextProto, EventContext<I>)) -> Self {
        let (ctx_proto, event_ctx) = value;
        SnekHandlerContext {
            output_ref: event_ctx.output_ref,
            metadata: event_ctx.metadata,
            consumed_utxos: event_ctx.consumed_utxos,
            consumed_identifiers: event_ctx.consumed_identifiers,
            produced_identifiers: event_ctx.produced_identifiers,
            executor_cred: ctx_proto.executor_cred,
            scripts: ctx_proto.scripts,
            bounds: ctx_proto.validation_rules,
            adhoc_fee_structure: ctx_proto.adhoc_fee_structure,
            auth_verification_key: ctx_proto.auth_verification_key,
        }
    }
}

impl<I: Copy> Has<AuthVerificationKey> for SnekHandlerContext<I> {
    fn select<U: IsEqual<AuthVerificationKey>>(&self) -> AuthVerificationKey {
        self.auth_verification_key
    }
}

impl<I: Copy> Has<Option<Metadata>> for SnekHandlerContext<I> {
    fn select<U: IsEqual<Option<Metadata>>>(&self) -> Option<Metadata> {
        self.metadata.clone()
    }
}

impl<I: Copy> Has<LimitOrderValidation> for SnekHandlerContext<I> {
    fn select<U: IsEqual<LimitOrderValidation>>(&self) -> LimitOrderValidation {
        self.bounds.limit_order
    }
}

impl<I: Copy> Has<DepositOrderValidation> for SnekHandlerContext<I> {
    fn select<U: IsEqual<DepositOrderValidation>>(&self) -> DepositOrderValidation {
        self.bounds.deposit_order
    }
}

impl<I: Copy> Has<RedeemOrderValidation> for SnekHandlerContext<I> {
    fn select<U: IsEqual<RedeemOrderValidation>>(&self) -> RedeemOrderValidation {
        self.bounds.redeem_order
    }
}

impl<I: Copy> Has<PoolValidation> for SnekHandlerContext<I> {
    fn select<U: IsEqual<PoolValidation>>(&self) -> PoolValidation {
        self.bounds.pool
    }
}

impl<I: Copy> Has<ConsumedInputs> for SnekHandlerContext<I> {
    fn select<U: IsEqual<ConsumedInputs>>(&self) -> ConsumedInputs {
        self.consumed_utxos
    }
}

impl<I: Copy> Has<ConsumedIdentifiers<I>> for SnekHandlerContext<I> {
    fn select<U: IsEqual<ConsumedIdentifiers<I>>>(&self) -> ConsumedIdentifiers<I> {
        self.consumed_identifiers
    }
}

impl<I: Copy> Has<ProducedIdentifiers<I>> for SnekHandlerContext<I> {
    fn select<U: IsEqual<ProducedIdentifiers<I>>>(&self) -> ProducedIdentifiers<I> {
        self.produced_identifiers
    }
}

impl<I: Copy> Has<DeployedScriptInfo<{ ConstFnPoolV1 as u8 }>> for SnekHandlerContext<I> {
    fn select<U: IsEqual<DeployedScriptInfo<{ ConstFnPoolV1 as u8 }>>>(
        &self,
    ) -> DeployedScriptInfo<{ ConstFnPoolV1 as u8 }> {
        self.scripts.const_fn_pool_v1.clone()
    }
}

impl<I: Copy> Has<DeployedScriptInfo<{ ConstFnPoolV2 as u8 }>> for SnekHandlerContext<I> {
    fn select<U: IsEqual<DeployedScriptInfo<{ ConstFnPoolV2 as u8 }>>>(
        &self,
    ) -> DeployedScriptInfo<{ ConstFnPoolV2 as u8 }> {
        self.scripts.const_fn_pool_v2.clone()
    }
}

impl<I: Copy> Has<DeployedScriptInfo<{ ConstFnPoolFeeSwitch as u8 }>> for SnekHandlerContext<I> {
    fn select<U: IsEqual<DeployedScriptInfo<{ ConstFnPoolFeeSwitch as u8 }>>>(
        &self,
    ) -> DeployedScriptInfo<{ ConstFnPoolFeeSwitch as u8 }> {
        self.scripts.const_fn_pool_fee_switch.clone()
    }
}

impl<I: Copy> Has<DeployedScriptInfo<{ ConstFnPoolFeeSwitchV2 as u8 }>> for SnekHandlerContext<I> {
    fn select<U: IsEqual<DeployedScriptInfo<{ ConstFnPoolFeeSwitchV2 as u8 }>>>(
        &self,
    ) -> DeployedScriptInfo<{ ConstFnPoolFeeSwitchV2 as u8 }> {
        self.scripts.const_fn_pool_fee_switch_v2.clone()
    }
}

impl<I: Copy> Has<DeployedScriptInfo<{ ConstFnPoolFeeSwitchBiDirFee as u8 }>> for SnekHandlerContext<I> {
    fn select<U: IsEqual<DeployedScriptInfo<{ ConstFnPoolFeeSwitchBiDirFee as u8 }>>>(
        &self,
    ) -> DeployedScriptInfo<{ ConstFnPoolFeeSwitchBiDirFee as u8 }> {
        self.scripts.const_fn_pool_fee_switch_bidir_fee.clone()
    }
}

impl<I: Copy> Has<DeployedScriptInfo<{ ConstFnPoolSwap as u8 }>> for SnekHandlerContext<I> {
    fn select<U: IsEqual<DeployedScriptInfo<{ ConstFnPoolSwap as u8 }>>>(
        &self,
    ) -> DeployedScriptInfo<{ ConstFnPoolSwap as u8 }> {
        self.scripts.const_fn_pool_swap.clone()
    }
}

impl<I: Copy> Has<DeployedScriptInfo<{ ConstFnPoolDeposit as u8 }>> for SnekHandlerContext<I> {
    fn select<U: IsEqual<DeployedScriptInfo<{ ConstFnPoolDeposit as u8 }>>>(
        &self,
    ) -> DeployedScriptInfo<{ ConstFnPoolDeposit as u8 }> {
        self.scripts.const_fn_pool_deposit.clone()
    }
}

impl<I: Copy> Has<DeployedScriptInfo<{ ConstFnPoolRedeem as u8 }>> for SnekHandlerContext<I> {
    fn select<U: IsEqual<DeployedScriptInfo<{ ConstFnPoolRedeem as u8 }>>>(
        &self,
    ) -> DeployedScriptInfo<{ ConstFnPoolRedeem as u8 }> {
        self.scripts.const_fn_pool_redeem.clone()
    }
}

impl<I: Copy> Has<DeployedScriptInfo<{ ConstFnFeeSwitchPoolSwap as u8 }>> for SnekHandlerContext<I> {
    fn select<U: IsEqual<DeployedScriptInfo<{ ConstFnFeeSwitchPoolSwap as u8 }>>>(
        &self,
    ) -> DeployedScriptInfo<{ ConstFnFeeSwitchPoolSwap as u8 }> {
        self.scripts.const_fn_fee_switch_pool_swap.clone()
    }
}

impl<I: Copy> Has<DeployedScriptInfo<{ ConstFnFeeSwitchPoolDeposit as u8 }>> for SnekHandlerContext<I> {
    fn select<U: IsEqual<DeployedScriptInfo<{ ConstFnFeeSwitchPoolDeposit as u8 }>>>(
        &self,
    ) -> DeployedScriptInfo<{ ConstFnFeeSwitchPoolDeposit as u8 }> {
        self.scripts.const_fn_fee_switch_pool_deposit.clone()
    }
}

impl<I: Copy> Has<DeployedScriptInfo<{ ConstFnFeeSwitchPoolRedeem as u8 }>> for SnekHandlerContext<I> {
    fn select<U: IsEqual<DeployedScriptInfo<{ ConstFnFeeSwitchPoolRedeem as u8 }>>>(
        &self,
    ) -> DeployedScriptInfo<{ ConstFnFeeSwitchPoolRedeem as u8 }> {
        self.scripts.const_fn_fee_switch_pool_redeem.clone()
    }
}

impl<I: Copy> Has<DeployedScriptInfo<{ BalanceFnPoolV1 as u8 }>> for SnekHandlerContext<I> {
    fn select<U: IsEqual<DeployedScriptInfo<{ BalanceFnPoolV1 as u8 }>>>(
        &self,
    ) -> DeployedScriptInfo<{ BalanceFnPoolV1 as u8 }> {
        self.scripts.balance_fn_pool_v1.clone()
    }
}

impl<I: Copy> Has<DeployedScriptInfo<{ BalanceFnPoolV2 as u8 }>> for SnekHandlerContext<I> {
    fn select<U: IsEqual<DeployedScriptInfo<{ BalanceFnPoolV2 as u8 }>>>(
        &self,
    ) -> DeployedScriptInfo<{ BalanceFnPoolV2 as u8 }> {
        self.scripts.balance_fn_pool_v2.clone()
    }
}

impl<I: Copy> Has<DeployedScriptInfo<{ BalanceFnPoolRedeem as u8 }>> for SnekHandlerContext<I> {
    fn select<U: IsEqual<DeployedScriptInfo<{ BalanceFnPoolRedeem as u8 }>>>(
        &self,
    ) -> DeployedScriptInfo<{ BalanceFnPoolRedeem as u8 }> {
        self.scripts.balance_fn_pool_redeem.clone()
    }
}

impl<I: Copy> Has<DeployedScriptInfo<{ BalanceFnPoolDeposit as u8 }>> for SnekHandlerContext<I> {
    fn select<U: IsEqual<DeployedScriptInfo<{ BalanceFnPoolDeposit as u8 }>>>(
        &self,
    ) -> DeployedScriptInfo<{ BalanceFnPoolDeposit as u8 }> {
        self.scripts.balance_fn_pool_deposit.clone()
    }
}

impl<I: Copy> Has<DeployedScriptInfo<{ LimitOrderV1 as u8 }>> for SnekHandlerContext<I> {
    fn select<U: IsEqual<DeployedScriptInfo<{ LimitOrderV1 as u8 }>>>(
        &self,
    ) -> DeployedScriptInfo<{ LimitOrderV1 as u8 }> {
        self.scripts.limit_order.clone()
    }
}

impl<I: Copy> Has<DeployedScriptInfo<{ LimitOrderWitnessV1 as u8 }>> for SnekHandlerContext<I> {
    fn select<U: IsEqual<DeployedScriptInfo<{ LimitOrderWitnessV1 as u8 }>>>(
        &self,
    ) -> DeployedScriptInfo<{ LimitOrderWitnessV1 as u8 }> {
        self.scripts.limit_order_witness.clone()
    }
}

impl<I: Copy> Has<DeployedScriptInfo<{ StableFnPoolT2T as u8 }>> for SnekHandlerContext<I> {
    fn select<U: IsEqual<DeployedScriptInfo<{ StableFnPoolT2T as u8 }>>>(
        &self,
    ) -> DeployedScriptInfo<{ StableFnPoolT2T as u8 }> {
        self.scripts.stable_fn_pool_t2t.clone()
    }
}

impl<I: Copy> Has<DeployedScriptInfo<{ StableFnPoolT2TDeposit as u8 }>> for SnekHandlerContext<I> {
    fn select<U: IsEqual<DeployedScriptInfo<{ StableFnPoolT2TDeposit as u8 }>>>(
        &self,
    ) -> DeployedScriptInfo<{ StableFnPoolT2TDeposit as u8 }> {
        self.scripts.stable_fn_pool_t2t_deposit.clone()
    }
}

impl<I: Copy> Has<DeployedScriptInfo<{ StableFnPoolT2TRedeem as u8 }>> for SnekHandlerContext<I> {
    fn select<U: IsEqual<DeployedScriptInfo<{ StableFnPoolT2TRedeem as u8 }>>>(
        &self,
    ) -> DeployedScriptInfo<{ StableFnPoolT2TRedeem as u8 }> {
        self.scripts.stable_fn_pool_t2t_redeem.clone()
    }
}
impl<I: Copy> Has<DeployedScriptInfo<{ DegenQuadraticPoolV1 as u8 }>> for SnekHandlerContext<I> {
    fn select<U: IsEqual<DeployedScriptInfo<{ DegenQuadraticPoolV1 as u8 }>>>(
        &self,
    ) -> DeployedScriptInfo<{ DegenQuadraticPoolV1 as u8 }> {
        self.scripts.degen_fn_pool_v1.clone()
    }
}

impl<I: Copy> Has<AdhocFeeStructure> for SnekHandlerContext<I> {
    fn select<U: IsEqual<AdhocFeeStructure>>(&self) -> AdhocFeeStructure {
        self.adhoc_fee_structure
    }
}

impl<I: Copy> Has<OutputRef> for SnekHandlerContext<I> {
    fn select<U: IsEqual<OutputRef>>(&self) -> OutputRef {
        self.output_ref
    }
}

impl<I: Copy> Has<OperatorCred> for SnekHandlerContext<I> {
    fn select<U: IsEqual<OperatorCred>>(&self) -> OperatorCred {
        self.executor_cred
    }
}
