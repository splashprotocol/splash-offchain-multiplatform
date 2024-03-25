use cml_chain::PolicyId;
use cml_crypto::ScriptHash;
use cml_multi_era::babbage::BabbageTransactionOutput;
use either::Either;
use log::trace;

use bloom_offchain::execution_engine::bundled::Bundled;
use spectrum_cardano_lib::output::FinalizedTxOut;
use spectrum_cardano_lib::OutputRef;
use spectrum_offchain::data::order::SpecializedOrder;
use spectrum_offchain::data::{Baked, EntitySnapshot, Has, Stable, Tradable};
use spectrum_offchain::ledger::TryFromLedger;
use spectrum_offchain_cardano::creds::OperatorCred;
use spectrum_offchain_cardano::data::order::ClassicalAMMOrder;
use spectrum_offchain_cardano::data::pair::PairId;
use spectrum_offchain_cardano::data::pool::AnyPool;
use spectrum_offchain_cardano::deployment::DeployedScriptHash;
use spectrum_offchain_cardano::deployment::ProtocolValidator::{
    BalanceFnPoolV1, ConstFnPoolDeposit, ConstFnPoolFeeSwitch, ConstFnPoolFeeSwitchBiDirFee,
    ConstFnPoolRedeem, ConstFnPoolSwap, ConstFnPoolV1, ConstFnPoolV2, LimitOrderV1,
};

use crate::orders::AnyOrder;

pub mod context;
pub mod entity_index;
pub mod handler;
pub mod order_index;

#[repr(transparent)]
#[derive(Debug, Clone)]
pub struct AtomicCardanoEntity(pub Bundled<ClassicalAMMOrder, FinalizedTxOut>);

impl SpecializedOrder for AtomicCardanoEntity {
    type TOrderId = OutputRef;
    type TPoolId = ScriptHash;

    fn get_self_ref(&self) -> Self::TOrderId {
        self.0.get_self_ref()
    }

    fn get_pool_ref(&self) -> Self::TPoolId {
        self.0.get_pool_ref()
    }
}

impl<C> TryFromLedger<BabbageTransactionOutput, C> for AtomicCardanoEntity
where
    C: Copy
        + Has<OperatorCred>
        + Has<OutputRef>
        + Has<DeployedScriptHash<{ ConstFnPoolSwap as u8 }>>
        + Has<DeployedScriptHash<{ ConstFnPoolDeposit as u8 }>>
        + Has<DeployedScriptHash<{ ConstFnPoolRedeem as u8 }>>,
{
    fn try_from_ledger(repr: &BabbageTransactionOutput, ctx: &C) -> Option<Self> {
        trace!(target: "offchain", "AtomicCardanoEntity::try_from_ledger");
        ClassicalAMMOrder::try_from_ledger(repr, ctx).map(|inner| {
            Self(Bundled(
                inner,
                FinalizedTxOut::new(repr.clone(), ctx.select::<OutputRef>()),
            ))
        })
    }
}

#[repr(transparent)]
#[derive(Debug, Clone)]
pub struct EvolvingCardanoEntity(
    pub Bundled<Either<Baked<AnyOrder, OutputRef>, Baked<AnyPool, OutputRef>>, FinalizedTxOut>,
);

impl Stable for EvolvingCardanoEntity {
    type StableId = PolicyId;
    fn stable_id(&self) -> Self::StableId {
        self.0.stable_id()
    }
    fn is_quasi_permanent(&self) -> bool {
        self.0.is_quasi_permanent()
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

impl<C> TryFromLedger<BabbageTransactionOutput, C> for EvolvingCardanoEntity
where
    C: Copy
        + Has<OperatorCred>
        + Has<OutputRef>
        + Has<DeployedScriptHash<{ ConstFnPoolV1 as u8 }>>
        + Has<DeployedScriptHash<{ ConstFnPoolV2 as u8 }>>
        + Has<DeployedScriptHash<{ ConstFnPoolFeeSwitch as u8 }>>
        + Has<DeployedScriptHash<{ ConstFnPoolFeeSwitchBiDirFee as u8 }>>
        + Has<DeployedScriptHash<{ BalanceFnPoolV1 as u8 }>>
        + Has<DeployedScriptHash<{ LimitOrderV1 as u8 }>>,
{
    fn try_from_ledger(repr: &BabbageTransactionOutput, ctx: &C) -> Option<Self> {
        trace!(target: "offchain", "CardanoEntity::try_from_ledger");
        <Either<Baked<AnyOrder, OutputRef>, Baked<AnyPool, OutputRef>>>::try_from_ledger(repr, ctx).map(
            |inner| {
                Self(Bundled(
                    inner,
                    FinalizedTxOut::new(repr.clone(), ctx.select::<OutputRef>()),
                ))
            },
        )
    }
}
