use bloom_offchain_cardano::event_sink::context::EventContext;
use bloom_offchain_cardano::orders::adhoc::AdhocFeeStructure;
use bloom_offchain_cardano::orders::limit::{BeaconMode, LimitOrderValidation};
use cml_chain::auxdata::Metadata;
use type_equalities::IsEqual;

use crate::snek_protocol_deployment::SnekProtocolScriptHashes;
use crate::snek_validation_rules::SnekValidationRules;
use spectrum_cardano_lib::OutputRef;
use spectrum_offchain::domain::Has;
use spectrum_offchain_cardano::creds::OperatorCred;
use spectrum_offchain_cardano::data::pool::PoolValidation;
use spectrum_offchain_cardano::deployment::DeployedScriptInfo;
use spectrum_offchain_cardano::deployment::ProtocolValidator::{
    DegenQuadraticPoolV1, LimitOrderV1, LimitOrderWitnessV1,
};
use spectrum_offchain_cardano::handler_context::{
    AuthVerificationKey, ConsumedIdentifiers, ConsumedInputs, Mints, ProducedIdentifiers,
};

#[derive(Copy, Clone, Debug)]
pub struct SnekHandlerContextProto {
    pub executor_cred: OperatorCred,
    pub scripts: SnekProtocolScriptHashes,
    pub validation_rules: SnekValidationRules,
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
    pub scripts: SnekProtocolScriptHashes,
    pub bounds: SnekValidationRules,
    pub adhoc_fee_structure: AdhocFeeStructure,
    pub auth_verification_key: AuthVerificationKey,
    pub mints: Option<Mints>,
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
            mints: event_ctx.mints,
        }
    }
}

impl<I: Copy> Has<Option<Mints>> for SnekHandlerContext<I> {
    fn select<U: IsEqual<Option<Mints>>>(&self) -> Option<Mints> {
        self.mints
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

impl<I: Copy> Has<BeaconMode> for SnekHandlerContext<I> {
    fn select<U: IsEqual<BeaconMode>>(&self) -> BeaconMode {
        BeaconMode::Adhoc
    }
}
