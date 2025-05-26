use cml_chain::auxdata::Metadata;
use cml_chain::transaction::TransactionOutput;
use either::Either;

use bloom_offchain::execution_engine::bundled::Bundled;
use bloom_offchain_cardano::orders::adhoc::{AdhocFeeStructure, AdhocOrder};
use bloom_offchain_cardano::orders::limit::LimitOrderValidation;
use spectrum_cardano_lib::output::FinalizedTxOut;
use spectrum_cardano_lib::{OutputRef, Token};
use spectrum_offchain::domain::{Baked, EntitySnapshot, Has, SeqState, Stable, Tradable};
use spectrum_offchain::ledger::TryFromLedger;
use spectrum_offchain_cardano::creds::OperatorCred;
use spectrum_offchain_cardano::data::pair::PairId;
use spectrum_offchain_cardano::data::pool::PoolValidation;
use spectrum_offchain_cardano::data::quadratic_pool::QuadraticPool;
use spectrum_offchain_cardano::deployment::DeployedScriptInfo;
use spectrum_offchain_cardano::deployment::ProtocolValidator::{DegenQuadraticPoolV1, InstantOrderV1};
use spectrum_offchain_cardano::handler_context::{
    AddedPaymentDestinations, AllowedAdditionalPaymentDestinations, ConsumedIdentifiers, ConsumedInputs,
    Mints, ProducedIdentifiers,
};

#[repr(transparent)]
#[derive(Debug, Clone)]
pub struct EvolvingCardanoEntity(
    pub Bundled<Either<Baked<AdhocOrder, OutputRef>, Baked<QuadraticPool, OutputRef>>, FinalizedTxOut>,
);

impl Stable for EvolvingCardanoEntity {
    type StableId = Token;
    fn stable_id(&self) -> Self::StableId {
        self.0.stable_id()
    }
    fn is_quasi_permanent(&self) -> bool {
        self.0.is_quasi_permanent()
    }
}

impl SeqState for EvolvingCardanoEntity {
    fn is_initial(&self) -> bool {
        self.0.is_initial()
    }
}

impl EntitySnapshot for EvolvingCardanoEntity {
    type Version = OutputRef;
    fn version(&self) -> Self::Version {
        self.0.version()
    }
}

impl Tradable for EvolvingCardanoEntity {
    type PairId = PairId;
    fn pair_id(&self) -> Self::PairId {
        self.0.pair_id()
    }
}

impl<C> TryFromLedger<TransactionOutput, C> for EvolvingCardanoEntity
where
    C: Clone
        + Has<OperatorCred>
        + Has<OutputRef>
        + Has<Option<Metadata>>
        + Has<ConsumedInputs>
        + Has<ConsumedIdentifiers<Token>>
        + Has<ProducedIdentifiers<Token>>
        + Has<AddedPaymentDestinations>
        + Has<AllowedAdditionalPaymentDestinations>
        + Has<DeployedScriptInfo<{ InstantOrderV1 as u8 }>>
        + Has<DeployedScriptInfo<{ DegenQuadraticPoolV1 as u8 }>>
        + Has<LimitOrderValidation>
        + Has<PoolValidation>
        + Has<AdhocFeeStructure>
        + Has<Option<Mints>>,
{
    fn try_from_ledger(repr: &TransactionOutput, ctx: &C) -> Option<Self> {
        <Either<Baked<AdhocOrder, OutputRef>, Baked<QuadraticPool, OutputRef>>>::try_from_ledger(repr, ctx)
            .map(|inner| {
                Self(Bundled(
                    inner,
                    FinalizedTxOut::new(repr.clone(), ctx.select::<OutputRef>()),
                ))
            })
    }
}
