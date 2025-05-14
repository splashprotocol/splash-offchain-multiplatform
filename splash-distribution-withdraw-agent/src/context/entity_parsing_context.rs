use cml_crypto::Ed25519KeyHash;
use spectrum_cardano_lib::OutputRef;
use spectrum_offchain::domain::Has;
use spectrum_offchain_cardano::deployment::DeployedScriptInfo;
use splash_dao_offchain::deployment::ProtocolValidator;
use splash_dao_offchain::protocol_config::{
    FarmAuthPolicy, NotOutputRefNorSlotNumber, PermManagerAuthPolicy, SplashPolicy,
};
use splash_dao_offchain::routines::Slot;
use splash_lp_indexer::config::HarvestLimits;
use type_equalities::IsEqual;
use crate::onchain::event::BufferWalletAddress;

pub struct EntityParsingContext<AppContext> {
    pub timed_output_ref: OutputRef,
    pub transaction_signatures: Vec<Ed25519KeyHash>,
    pub slot: Slot,
    pub app_context: AppContext,
}

impl<AppContext> Has<OutputRef> for EntityParsingContext<AppContext> {
    fn select<U: IsEqual<OutputRef>>(&self) -> OutputRef {
        self.timed_output_ref
    }
}

impl<AppContext> Has<Vec<Ed25519KeyHash>> for EntityParsingContext<AppContext> {
    fn select<U: IsEqual<Vec<Ed25519KeyHash>>>(&self) -> Vec<Ed25519KeyHash> {
        self.transaction_signatures.clone()
    }
}

impl<AppContext> Has<Slot> for EntityParsingContext<AppContext> {
    fn select<U: IsEqual<Slot>>(&self) -> Slot {
        self.slot
    }
}

impl<AppContext> Has<PermManagerAuthPolicy> for EntityParsingContext<AppContext>
where
    AppContext: Has<PermManagerAuthPolicy>,
{
    fn select<U: IsEqual<PermManagerAuthPolicy>>(&self) -> PermManagerAuthPolicy {
        self.app_context.get()
    }
}

impl<AppContext> Has<FarmAuthPolicy> for EntityParsingContext<AppContext>
where
    AppContext: Has<FarmAuthPolicy>,
{
    fn select<U: IsEqual<FarmAuthPolicy>>(&self) -> FarmAuthPolicy {
        self.app_context.get()
    }
}

impl<AppContext> Has<SplashPolicy> for EntityParsingContext<AppContext>
where
    AppContext: Has<SplashPolicy>,
{
    fn select<U: IsEqual<SplashPolicy>>(&self) -> SplashPolicy {
        self.app_context.get()
    }
}

impl<AppContext> Has<HarvestLimits> for EntityParsingContext<AppContext>
where
    AppContext: Has<HarvestLimits>,
{
    fn select<U: IsEqual<HarvestLimits>>(&self) -> HarvestLimits {
        self.app_context.get()
    }
}

impl<AppContext> Has<DeployedScriptInfo<{ ProtocolValidator::SmartFarm as u8 }>>
    for EntityParsingContext<AppContext>
where
    AppContext: Has<DeployedScriptInfo<{ ProtocolValidator::SmartFarm as u8 }>>,
{
    fn select<U: IsEqual<DeployedScriptInfo<{ ProtocolValidator::SmartFarm as u8 }>>>(
        &self,
    ) -> DeployedScriptInfo<{ ProtocolValidator::SmartFarm as u8 }> {
        self.app_context.get()
    }
}

impl<AppContext> Has<DeployedScriptInfo<{ ProtocolValidator::HarvestOrder as u8 }>>
for EntityParsingContext<AppContext>
where
    AppContext: Has<DeployedScriptInfo<{ ProtocolValidator::HarvestOrder as u8 }>>,
{
    fn select<U: IsEqual<DeployedScriptInfo<{ ProtocolValidator::HarvestOrder as u8 }>>>(
        &self,
    ) -> DeployedScriptInfo<{ ProtocolValidator::HarvestOrder as u8 }> {
        self.app_context.get()
    }
}

impl<AppContext> Has<DeployedScriptInfo<{ ProtocolValidator::PermManager as u8 }>>
for EntityParsingContext<AppContext>
where
    AppContext: Has<DeployedScriptInfo<{ ProtocolValidator::PermManager as u8 }>>,
{
    fn select<U: IsEqual<DeployedScriptInfo<{ ProtocolValidator::PermManager as u8 }>>>(
        &self,
    ) -> DeployedScriptInfo<{ ProtocolValidator::PermManager as u8 }> {
        self.app_context.get()
    }
}

impl<AppContext> Has<BufferWalletAddress> for EntityParsingContext<AppContext>
where
    AppContext: Has<BufferWalletAddress>,
{
    fn select<U: IsEqual<BufferWalletAddress>>(&self) -> BufferWalletAddress {
        self.app_context.get()
    }
}

// + Has<DeployedScriptInfo<{ ProtocolValidator::SmartFarm as u8 }>>
// + Has<DeployedScriptInfo<{ ProtocolValidator::HarvestOrder as u8 }>>
// + Has<BufferWalletAddress>
// + Has<SplashPolicy>,

// impl<AppContext, H> Has<H> for EntityParsingContext<AppContext>
// where
//     AppContext: Has<H>
// {
//     fn select<U: IsEqual<H>>(&self) -> H {
//         self.app_context.select::<U>()
//     }
// }
