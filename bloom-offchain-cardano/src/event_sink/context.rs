use cml_chain::auxdata::Metadata;
use type_equalities::IsEqual;

use bloom_offchain::execution_engine::types::Time;
use spectrum_cardano_lib::{NetworkId, OutputRef};
use spectrum_offchain::domain::Has;
use spectrum_offchain_cardano::creds::OperatorCred;
use spectrum_offchain_cardano::data::dao_request::{DAOContext, DAOV1ActionOrderValidation};
use spectrum_offchain_cardano::data::deposit::DepositOrderValidation;
use spectrum_offchain_cardano::data::pool::PoolValidation;
use spectrum_offchain_cardano::data::redeem::RedeemOrderValidation;
use spectrum_offchain_cardano::data::royalty_withdraw_request::RoyaltyWithdrawOrderValidation;
use spectrum_offchain_cardano::deployment::ProtocolValidator::{
    BalanceFnPoolDeposit, BalanceFnPoolRedeem, BalanceFnPoolV1, BalanceFnPoolV2, ConstFnFeeSwitchPoolDeposit,
    ConstFnFeeSwitchPoolRedeem, ConstFnFeeSwitchPoolSwap, ConstFnPoolDeposit, ConstFnPoolFeeSwitch,
    ConstFnPoolFeeSwitchBiDirFee, ConstFnPoolFeeSwitchV2, ConstFnPoolRedeem, ConstFnPoolSwap, ConstFnPoolV1,
    ConstFnPoolV2, LimitOrderV1, LimitOrderWitnessV1, RoyaltyPoolDAOV1Request, RoyaltyPoolV1,
    RoyaltyPoolV1Deposit, RoyaltyPoolV1LedgerFixed, RoyaltyPoolV1Redeem, RoyaltyPoolV1RoyaltyWithdrawRequest,
    RoyaltyPoolV2, RoyaltyPoolV2DAOV1Request, RoyaltyPoolV2Deposit, RoyaltyPoolV2Redeem,
    RoyaltyPoolV2RoyaltyWithdrawRequest, StableFnPoolT2T, StableFnPoolT2TDeposit, StableFnPoolT2TRedeem,
};
use spectrum_offchain_cardano::deployment::{DeployedScriptInfo, ProtocolScriptHashes};
use spectrum_offchain_cardano::handler_context::{
    AddedPaymentDestinations, ConsumedIdentifiers, ConsumedInputs, Mints, ProducedIdentifiers,
};

use crate::graduation::{
    GraduatedPoolFeeConfig, GraduatedSplashPoolStore, GraduationStateRocksDb, SnekPoolInputTracker,
    SnekPoolScriptHashes,
};
use crate::orders::adhoc::AdhocFeeStructure;
use crate::orders::auction::AuctionOrderRegistry;
use crate::orders::limit::LimitOrderValidation;
use crate::validation_rules::ValidationRules;

pub struct EventContext<I: Copy> {
    pub output_ref: OutputRef,
    pub metadata: Option<Metadata>,
    pub consumed_utxos: ConsumedInputs,
    pub consumed_identifiers: ConsumedIdentifiers<I>,
    pub produced_identifiers: ProducedIdentifiers<I>,
    pub added_payment_destinations: AddedPaymentDestinations,
    pub mints: Option<Mints>,
    pub posix_time: Option<u64>,
}

#[derive(Clone, Debug)]
pub struct HandlerContextProto {
    pub executor_cred: OperatorCred,
    pub scripts: ProtocolScriptHashes,
    pub validation_rules: ValidationRules,
    pub adhoc_fee_structure: AdhocFeeStructure,
    pub dao_context: DAOContext,
    pub graduated_pool_fee_config: GraduatedPoolFeeConfig,
    pub snek_pool_script_hashes: SnekPoolScriptHashes,
    pub graduated_pool_store: GraduatedSplashPoolStore,
    pub snek_pool_input_tracker: SnekPoolInputTracker,
    pub graduation_state: Option<GraduationStateRocksDb>,
    pub auction_order_registry: AuctionOrderRegistry,
    pub network_id: NetworkId,
}

#[derive(Clone, Debug)]
pub struct HandlerContext<I: Copy> {
    pub output_ref: OutputRef,
    pub consumed_utxos: ConsumedInputs,
    pub consumed_identifiers: ConsumedIdentifiers<I>,
    pub produced_identifiers: ProducedIdentifiers<I>,
    pub executor_cred: OperatorCred,
    pub scripts: ProtocolScriptHashes,
    pub bounds: ValidationRules,
    pub adhoc_fee_structure: AdhocFeeStructure,
    pub dao_context: DAOContext,
    pub graduated_pool_fee_config: GraduatedPoolFeeConfig,
    pub snek_pool_script_hashes: SnekPoolScriptHashes,
    pub graduated_pool_store: GraduatedSplashPoolStore,
    pub snek_pool_input_tracker: SnekPoolInputTracker,
    pub graduation_state: Option<GraduationStateRocksDb>,
    pub auction_order_registry: AuctionOrderRegistry,
    pub posix_time: Option<u64>,
    pub mints: Option<Mints>,
}

impl<I: Copy> From<(HandlerContextProto, EventContext<I>)> for HandlerContext<I> {
    fn from(value: (HandlerContextProto, EventContext<I>)) -> Self {
        let (ctx_proto, event_ctx) = value;
        HandlerContext {
            output_ref: event_ctx.output_ref,
            consumed_utxos: event_ctx.consumed_utxos,
            consumed_identifiers: event_ctx.consumed_identifiers,
            produced_identifiers: event_ctx.produced_identifiers,
            executor_cred: ctx_proto.executor_cred,
            scripts: ctx_proto.scripts,
            bounds: ctx_proto.validation_rules,
            adhoc_fee_structure: ctx_proto.adhoc_fee_structure,
            dao_context: ctx_proto.dao_context,
            graduated_pool_fee_config: ctx_proto.graduated_pool_fee_config,
            snek_pool_script_hashes: ctx_proto.snek_pool_script_hashes,
            graduated_pool_store: ctx_proto.graduated_pool_store,
            snek_pool_input_tracker: ctx_proto.snek_pool_input_tracker,
            graduation_state: ctx_proto.graduation_state,
            auction_order_registry: ctx_proto.auction_order_registry,
            posix_time: event_ctx.posix_time,
            mints: event_ctx.mints,
        }
    }
}

impl<I: Copy> Has<Option<Mints>> for HandlerContext<I> {
    fn select<U: IsEqual<Option<Mints>>>(&self) -> Option<Mints> {
        self.mints
    }
}

impl<I: Copy> Has<LimitOrderValidation> for HandlerContext<I> {
    fn select<U: IsEqual<LimitOrderValidation>>(&self) -> LimitOrderValidation {
        self.bounds.limit_order
    }
}

impl<I: Copy> Has<AuctionOrderRegistry> for HandlerContext<I> {
    fn select<U: IsEqual<AuctionOrderRegistry>>(&self) -> AuctionOrderRegistry {
        self.auction_order_registry.clone()
    }
}

impl<I: Copy> Has<Time> for HandlerContext<I> {
    fn select<U: IsEqual<Time>>(&self) -> Time {
        Time::from(self.posix_time.unwrap_or(0))
    }
}

impl<I: Copy> Has<DepositOrderValidation> for HandlerContext<I> {
    fn select<U: IsEqual<DepositOrderValidation>>(&self) -> DepositOrderValidation {
        self.bounds.deposit_order
    }
}

impl<I: Copy> Has<RoyaltyWithdrawOrderValidation> for HandlerContext<I> {
    fn select<U: IsEqual<RoyaltyWithdrawOrderValidation>>(&self) -> RoyaltyWithdrawOrderValidation {
        self.bounds.royalty_withdraw
    }
}

impl<I: Copy> Has<DAOV1ActionOrderValidation> for HandlerContext<I> {
    fn select<U: IsEqual<DAOV1ActionOrderValidation>>(&self) -> DAOV1ActionOrderValidation {
        self.bounds.dao_action
    }
}

impl<I: Copy> Has<RedeemOrderValidation> for HandlerContext<I> {
    fn select<U: IsEqual<RedeemOrderValidation>>(&self) -> RedeemOrderValidation {
        self.bounds.redeem_order
    }
}

impl<I: Copy> Has<PoolValidation> for HandlerContext<I> {
    fn select<U: IsEqual<PoolValidation>>(&self) -> PoolValidation {
        self.bounds.pool
    }
}

impl<I: Copy> Has<ConsumedInputs> for HandlerContext<I> {
    fn select<U: IsEqual<ConsumedInputs>>(&self) -> ConsumedInputs {
        self.consumed_utxos
    }
}

impl<I: Copy> Has<ConsumedIdentifiers<I>> for HandlerContext<I> {
    fn select<U: IsEqual<ConsumedIdentifiers<I>>>(&self) -> ConsumedIdentifiers<I> {
        self.consumed_identifiers
    }
}

impl<I: Copy> Has<ProducedIdentifiers<I>> for HandlerContext<I> {
    fn select<U: IsEqual<ProducedIdentifiers<I>>>(&self) -> ProducedIdentifiers<I> {
        self.produced_identifiers
    }
}

impl<I: Copy> Has<DeployedScriptInfo<{ ConstFnPoolV1 as u8 }>> for HandlerContext<I> {
    fn select<U: IsEqual<DeployedScriptInfo<{ ConstFnPoolV1 as u8 }>>>(
        &self,
    ) -> DeployedScriptInfo<{ ConstFnPoolV1 as u8 }> {
        self.scripts.const_fn_pool_v1.clone()
    }
}

impl<I: Copy> Has<DeployedScriptInfo<{ ConstFnPoolV2 as u8 }>> for HandlerContext<I> {
    fn select<U: IsEqual<DeployedScriptInfo<{ ConstFnPoolV2 as u8 }>>>(
        &self,
    ) -> DeployedScriptInfo<{ ConstFnPoolV2 as u8 }> {
        self.scripts.const_fn_pool_v2.clone()
    }
}

impl<I: Copy> Has<DeployedScriptInfo<{ ConstFnPoolFeeSwitch as u8 }>> for HandlerContext<I> {
    fn select<U: IsEqual<DeployedScriptInfo<{ ConstFnPoolFeeSwitch as u8 }>>>(
        &self,
    ) -> DeployedScriptInfo<{ ConstFnPoolFeeSwitch as u8 }> {
        self.scripts.const_fn_pool_fee_switch.clone()
    }
}

impl<I: Copy> Has<DeployedScriptInfo<{ ConstFnPoolFeeSwitchV2 as u8 }>> for HandlerContext<I> {
    fn select<U: IsEqual<DeployedScriptInfo<{ ConstFnPoolFeeSwitchV2 as u8 }>>>(
        &self,
    ) -> DeployedScriptInfo<{ ConstFnPoolFeeSwitchV2 as u8 }> {
        self.scripts.const_fn_pool_fee_switch_v2.clone()
    }
}

impl<I: Copy> Has<DeployedScriptInfo<{ ConstFnPoolFeeSwitchBiDirFee as u8 }>> for HandlerContext<I> {
    fn select<U: IsEqual<DeployedScriptInfo<{ ConstFnPoolFeeSwitchBiDirFee as u8 }>>>(
        &self,
    ) -> DeployedScriptInfo<{ ConstFnPoolFeeSwitchBiDirFee as u8 }> {
        self.scripts.const_fn_pool_fee_switch_bidir_fee.clone()
    }
}

impl<I: Copy> Has<DeployedScriptInfo<{ ConstFnPoolSwap as u8 }>> for HandlerContext<I> {
    fn select<U: IsEqual<DeployedScriptInfo<{ ConstFnPoolSwap as u8 }>>>(
        &self,
    ) -> DeployedScriptInfo<{ ConstFnPoolSwap as u8 }> {
        self.scripts.const_fn_pool_swap.clone()
    }
}

impl<I: Copy> Has<DeployedScriptInfo<{ ConstFnPoolDeposit as u8 }>> for HandlerContext<I> {
    fn select<U: IsEqual<DeployedScriptInfo<{ ConstFnPoolDeposit as u8 }>>>(
        &self,
    ) -> DeployedScriptInfo<{ ConstFnPoolDeposit as u8 }> {
        self.scripts.const_fn_pool_deposit.clone()
    }
}

impl<I: Copy> Has<DeployedScriptInfo<{ ConstFnPoolRedeem as u8 }>> for HandlerContext<I> {
    fn select<U: IsEqual<DeployedScriptInfo<{ ConstFnPoolRedeem as u8 }>>>(
        &self,
    ) -> DeployedScriptInfo<{ ConstFnPoolRedeem as u8 }> {
        self.scripts.const_fn_pool_redeem.clone()
    }
}

impl<I: Copy> Has<DeployedScriptInfo<{ ConstFnFeeSwitchPoolSwap as u8 }>> for HandlerContext<I> {
    fn select<U: IsEqual<DeployedScriptInfo<{ ConstFnFeeSwitchPoolSwap as u8 }>>>(
        &self,
    ) -> DeployedScriptInfo<{ ConstFnFeeSwitchPoolSwap as u8 }> {
        self.scripts.const_fn_fee_switch_pool_swap.clone()
    }
}

impl<I: Copy> Has<DeployedScriptInfo<{ ConstFnFeeSwitchPoolDeposit as u8 }>> for HandlerContext<I> {
    fn select<U: IsEqual<DeployedScriptInfo<{ ConstFnFeeSwitchPoolDeposit as u8 }>>>(
        &self,
    ) -> DeployedScriptInfo<{ ConstFnFeeSwitchPoolDeposit as u8 }> {
        self.scripts.const_fn_fee_switch_pool_deposit.clone()
    }
}

impl<I: Copy> Has<DeployedScriptInfo<{ ConstFnFeeSwitchPoolRedeem as u8 }>> for HandlerContext<I> {
    fn select<U: IsEqual<DeployedScriptInfo<{ ConstFnFeeSwitchPoolRedeem as u8 }>>>(
        &self,
    ) -> DeployedScriptInfo<{ ConstFnFeeSwitchPoolRedeem as u8 }> {
        self.scripts.const_fn_fee_switch_pool_redeem.clone()
    }
}

impl<I: Copy> Has<DeployedScriptInfo<{ BalanceFnPoolV1 as u8 }>> for HandlerContext<I> {
    fn select<U: IsEqual<DeployedScriptInfo<{ BalanceFnPoolV1 as u8 }>>>(
        &self,
    ) -> DeployedScriptInfo<{ BalanceFnPoolV1 as u8 }> {
        self.scripts.balance_fn_pool_v1.clone()
    }
}

impl<I: Copy> Has<DeployedScriptInfo<{ BalanceFnPoolV2 as u8 }>> for HandlerContext<I> {
    fn select<U: IsEqual<DeployedScriptInfo<{ BalanceFnPoolV2 as u8 }>>>(
        &self,
    ) -> DeployedScriptInfo<{ BalanceFnPoolV2 as u8 }> {
        self.scripts.balance_fn_pool_v2.clone()
    }
}

impl<I: Copy> Has<DeployedScriptInfo<{ BalanceFnPoolRedeem as u8 }>> for HandlerContext<I> {
    fn select<U: IsEqual<DeployedScriptInfo<{ BalanceFnPoolRedeem as u8 }>>>(
        &self,
    ) -> DeployedScriptInfo<{ BalanceFnPoolRedeem as u8 }> {
        self.scripts.balance_fn_pool_redeem.clone()
    }
}

impl<I: Copy> Has<DeployedScriptInfo<{ BalanceFnPoolDeposit as u8 }>> for HandlerContext<I> {
    fn select<U: IsEqual<DeployedScriptInfo<{ BalanceFnPoolDeposit as u8 }>>>(
        &self,
    ) -> DeployedScriptInfo<{ BalanceFnPoolDeposit as u8 }> {
        self.scripts.balance_fn_pool_deposit.clone()
    }
}

impl<I: Copy> Has<DeployedScriptInfo<{ LimitOrderV1 as u8 }>> for HandlerContext<I> {
    fn select<U: IsEqual<DeployedScriptInfo<{ LimitOrderV1 as u8 }>>>(
        &self,
    ) -> DeployedScriptInfo<{ LimitOrderV1 as u8 }> {
        self.scripts.limit_order.clone()
    }
}

impl<I: Copy> Has<DeployedScriptInfo<{ LimitOrderWitnessV1 as u8 }>> for HandlerContext<I> {
    fn select<U: IsEqual<DeployedScriptInfo<{ LimitOrderWitnessV1 as u8 }>>>(
        &self,
    ) -> DeployedScriptInfo<{ LimitOrderWitnessV1 as u8 }> {
        self.scripts.limit_order_witness.clone()
    }
}

impl<I: Copy> Has<DeployedScriptInfo<{ StableFnPoolT2T as u8 }>> for HandlerContext<I> {
    fn select<U: IsEqual<DeployedScriptInfo<{ StableFnPoolT2T as u8 }>>>(
        &self,
    ) -> DeployedScriptInfo<{ StableFnPoolT2T as u8 }> {
        self.scripts.stable_fn_pool_t2t.clone()
    }
}

impl<I: Copy> Has<DeployedScriptInfo<{ StableFnPoolT2TDeposit as u8 }>> for HandlerContext<I> {
    fn select<U: IsEqual<DeployedScriptInfo<{ StableFnPoolT2TDeposit as u8 }>>>(
        &self,
    ) -> DeployedScriptInfo<{ StableFnPoolT2TDeposit as u8 }> {
        self.scripts.stable_fn_pool_t2t_deposit.clone()
    }
}

impl<I: Copy> Has<DeployedScriptInfo<{ StableFnPoolT2TRedeem as u8 }>> for HandlerContext<I> {
    fn select<U: IsEqual<DeployedScriptInfo<{ StableFnPoolT2TRedeem as u8 }>>>(
        &self,
    ) -> DeployedScriptInfo<{ StableFnPoolT2TRedeem as u8 }> {
        self.scripts.stable_fn_pool_t2t_redeem.clone()
    }
}

impl<I: Copy> Has<DeployedScriptInfo<{ RoyaltyPoolV1 as u8 }>> for HandlerContext<I> {
    fn select<U: IsEqual<DeployedScriptInfo<{ RoyaltyPoolV1 as u8 }>>>(
        &self,
    ) -> DeployedScriptInfo<{ RoyaltyPoolV1 as u8 }> {
        self.scripts.royalty_pool_v1.clone()
    }
}

impl<I: Copy> Has<DeployedScriptInfo<{ RoyaltyPoolV1LedgerFixed as u8 }>> for HandlerContext<I> {
    fn select<U: IsEqual<DeployedScriptInfo<{ RoyaltyPoolV1LedgerFixed as u8 }>>>(
        &self,
    ) -> DeployedScriptInfo<{ RoyaltyPoolV1LedgerFixed as u8 }> {
        self.scripts.royalty_pool_v1_ledger_fixed.clone()
    }
}

impl<I: Copy> Has<DeployedScriptInfo<{ RoyaltyPoolV2 as u8 }>> for HandlerContext<I> {
    fn select<U: IsEqual<DeployedScriptInfo<{ RoyaltyPoolV2 as u8 }>>>(
        &self,
    ) -> DeployedScriptInfo<{ RoyaltyPoolV2 as u8 }> {
        self.scripts.royalty_pool_v2.clone()
    }
}

impl<I: Copy> Has<DeployedScriptInfo<{ RoyaltyPoolV1Deposit as u8 }>> for HandlerContext<I> {
    fn select<U: IsEqual<DeployedScriptInfo<{ RoyaltyPoolV1Deposit as u8 }>>>(
        &self,
    ) -> DeployedScriptInfo<{ RoyaltyPoolV1Deposit as u8 }> {
        self.scripts.royalty_pool_deposit.clone()
    }
}

impl<I: Copy> Has<DeployedScriptInfo<{ RoyaltyPoolV2Deposit as u8 }>> for HandlerContext<I> {
    fn select<U: IsEqual<DeployedScriptInfo<{ RoyaltyPoolV2Deposit as u8 }>>>(
        &self,
    ) -> DeployedScriptInfo<{ RoyaltyPoolV2Deposit as u8 }> {
        self.scripts.royalty_pool_deposit_v2.clone()
    }
}

impl<I: Copy> Has<DeployedScriptInfo<{ RoyaltyPoolV1Redeem as u8 }>> for HandlerContext<I> {
    fn select<U: IsEqual<DeployedScriptInfo<{ RoyaltyPoolV1Redeem as u8 }>>>(
        &self,
    ) -> DeployedScriptInfo<{ RoyaltyPoolV1Redeem as u8 }> {
        self.scripts.royalty_pool_redeem.clone()
    }
}

impl<I: Copy> Has<DeployedScriptInfo<{ RoyaltyPoolV2Redeem as u8 }>> for HandlerContext<I> {
    fn select<U: IsEqual<DeployedScriptInfo<{ RoyaltyPoolV2Redeem as u8 }>>>(
        &self,
    ) -> DeployedScriptInfo<{ RoyaltyPoolV2Redeem as u8 }> {
        self.scripts.royalty_pool_redeem_v2.clone()
    }
}

impl<I: Copy> Has<DeployedScriptInfo<{ RoyaltyPoolV1RoyaltyWithdrawRequest as u8 }>> for HandlerContext<I> {
    fn select<U: IsEqual<DeployedScriptInfo<{ RoyaltyPoolV1RoyaltyWithdrawRequest as u8 }>>>(
        &self,
    ) -> DeployedScriptInfo<{ RoyaltyPoolV1RoyaltyWithdrawRequest as u8 }> {
        self.scripts.royalty_pool_withdraw_request.clone()
    }
}

impl<I: Copy> Has<DeployedScriptInfo<{ RoyaltyPoolV2RoyaltyWithdrawRequest as u8 }>> for HandlerContext<I> {
    fn select<U: IsEqual<DeployedScriptInfo<{ RoyaltyPoolV2RoyaltyWithdrawRequest as u8 }>>>(
        &self,
    ) -> DeployedScriptInfo<{ RoyaltyPoolV2RoyaltyWithdrawRequest as u8 }> {
        self.scripts.royalty_pool_v2_withdraw_request.clone()
    }
}

impl<I: Copy> Has<DeployedScriptInfo<{ RoyaltyPoolDAOV1Request as u8 }>> for HandlerContext<I> {
    fn select<U: IsEqual<DeployedScriptInfo<{ RoyaltyPoolDAOV1Request as u8 }>>>(
        &self,
    ) -> DeployedScriptInfo<{ RoyaltyPoolDAOV1Request as u8 }> {
        self.scripts.royalty_pool_dao_request.clone()
    }
}

impl<I: Copy> Has<DeployedScriptInfo<{ RoyaltyPoolV2DAOV1Request as u8 }>> for HandlerContext<I> {
    fn select<U: IsEqual<DeployedScriptInfo<{ RoyaltyPoolV2DAOV1Request as u8 }>>>(
        &self,
    ) -> DeployedScriptInfo<{ RoyaltyPoolV2DAOV1Request as u8 }> {
        self.scripts.royalty_pool_v2_dao_request.clone()
    }
}

impl<I: Copy> Has<AdhocFeeStructure> for HandlerContext<I> {
    fn select<U: IsEqual<AdhocFeeStructure>>(&self) -> AdhocFeeStructure {
        self.adhoc_fee_structure
    }
}

impl<I: Copy> Has<OutputRef> for HandlerContext<I> {
    fn select<U: IsEqual<OutputRef>>(&self) -> OutputRef {
        self.output_ref
    }
}

impl<I: Copy> Has<OperatorCred> for HandlerContext<I> {
    fn select<U: IsEqual<OperatorCred>>(&self) -> OperatorCred {
        self.executor_cred
    }
}

impl<I: Copy> Has<DAOContext> for HandlerContext<I> {
    fn select<U: IsEqual<DAOContext>>(&self) -> DAOContext {
        self.dao_context.clone()
    }
}

impl<I: Copy> Has<GraduatedPoolFeeConfig> for HandlerContext<I> {
    fn select<U: IsEqual<GraduatedPoolFeeConfig>>(&self) -> GraduatedPoolFeeConfig {
        self.graduated_pool_fee_config
    }
}

impl<I: Copy> Has<GraduatedSplashPoolStore> for HandlerContext<I> {
    fn select<U: IsEqual<GraduatedSplashPoolStore>>(&self) -> GraduatedSplashPoolStore {
        self.graduated_pool_store.clone()
    }
}

impl<I: Copy> Has<SnekPoolInputTracker> for HandlerContext<I> {
    fn select<U: IsEqual<SnekPoolInputTracker>>(&self) -> SnekPoolInputTracker {
        self.snek_pool_input_tracker.clone()
    }
}
