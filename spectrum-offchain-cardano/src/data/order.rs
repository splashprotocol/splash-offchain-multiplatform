use std::fmt::{Display, Formatter};
use std::hash::{Hash, Hasher};

use cml_chain::builders::tx_builder::SignedTxBuilder;
use cml_chain::plutus::{ConstrPlutusData, PlutusData};
use cml_chain::utils::BigInt;
use cml_crypto::ScriptHash;
use cml_multi_era::babbage::BabbageTransactionOutput;

use bloom_offchain::execution_engine::bundled::Bundled;
use spectrum_cardano_lib::collateral::Collateral;
use spectrum_cardano_lib::output::FinalizedTxOut;
use spectrum_cardano_lib::{NetworkId, OutputRef};
use spectrum_offchain::backlog::data::{OrderWeight, Weighted};
use spectrum_offchain::data::order::{SpecializedOrder, UniqueOrder};
use spectrum_offchain::data::unique_entity::Predicted;
use spectrum_offchain::data::Has;
use spectrum_offchain::executor::{RunOrder, RunOrderError};
use spectrum_offchain::ledger::TryFromLedger;

use crate::creds::OperatorRewardAddress;
use crate::data::deposit::ClassicalOnChainDeposit;
use crate::data::limit_swap::ClassicalOnChainLimitSwap;
use crate::data::pool::{CFMMPool, RunAnyCFMMOrderOverPool};
use crate::data::redeem::ClassicalOnChainRedeem;
use crate::data::PoolId;
use crate::deployment::DeployedValidator;
use crate::deployment::ProtocolValidator::{
    ConstFnPoolDeposit, ConstFnPoolRedeem, ConstFnPoolSwap, ConstFnPoolV1, ConstFnPoolV2,
};

pub struct Input;

pub struct Output;

pub struct Base;

pub struct Quote;

pub struct PoolNft;

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct ClassicalOrder<Id, Ord> {
    pub id: Id,
    pub pool_id: PoolId,
    pub order: Ord,
}

impl<Id: Clone, Ord> Has<Id> for ClassicalOrder<Id, Ord> {
    fn get_labeled<U: type_equalities::IsEqual<Id>>(&self) -> Id {
        self.id.clone()
    }
}

pub enum ClassicalOrderAction {
    Apply,
}

impl ClassicalOrderAction {
    pub fn to_plutus_data(self) -> PlutusData {
        match self {
            ClassicalOrderAction::Apply => PlutusData::ConstrPlutusData(ConstrPlutusData::new(0, Vec::new())),
        }
    }
}

pub struct ClassicalOrderRedeemer {
    pub pool_input_index: u64,
    pub order_input_index: u64,
    pub output_index: u64,
    pub action: ClassicalOrderAction,
}

impl ClassicalOrderRedeemer {
    pub fn to_plutus_data(self) -> PlutusData {
        let action_pd = self.action.to_plutus_data();
        let pool_in_ix_pd = PlutusData::Integer(BigInt::from(self.pool_input_index));
        let order_in_ix_pd = PlutusData::Integer(BigInt::from(self.order_input_index));
        let out_ix_pd = PlutusData::Integer(BigInt::from(self.output_index));
        PlutusData::ConstrPlutusData(ConstrPlutusData::new(
            0,
            vec![pool_in_ix_pd, order_in_ix_pd, out_ix_pd, action_pd],
        ))
    }
}

#[derive(Debug, Clone)]
pub enum ClassicalAMMOrder {
    Swap(ClassicalOnChainLimitSwap),
    Deposit(ClassicalOnChainDeposit),
    Redeem(ClassicalOnChainRedeem),
}

impl Display for ClassicalAMMOrder {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str("ClassicalAMMOrder")
    }
}

impl Weighted for ClassicalAMMOrder {
    fn weight(&self) -> OrderWeight {
        match self {
            ClassicalAMMOrder::Swap(limit_swap) => OrderWeight::from(limit_swap.order.fee.0),
            ClassicalAMMOrder::Deposit(deposit) => OrderWeight::from(deposit.order.ex_fee),
            ClassicalAMMOrder::Redeem(redeem) => OrderWeight::from(redeem.order.ex_fee),
        }
    }
}

impl PartialEq for ClassicalAMMOrder {
    fn eq(&self, other: &Self) -> bool {
        <Self as UniqueOrder>::get_self_ref(self).eq(&<Self as UniqueOrder>::get_self_ref(other))
    }
}

impl Eq for ClassicalAMMOrder {}

impl Hash for ClassicalAMMOrder {
    fn hash<H: Hasher>(&self, state: &mut H) {
        <Self as UniqueOrder>::get_self_ref(self).hash(state)
    }
}

impl SpecializedOrder for ClassicalAMMOrder {
    type TOrderId = OutputRef;
    type TPoolId = ScriptHash;

    fn get_self_ref(&self) -> Self::TOrderId {
        match self {
            ClassicalAMMOrder::Swap(swap) => swap.id.into(),
            ClassicalAMMOrder::Deposit(dep) => dep.id.into(),
            ClassicalAMMOrder::Redeem(red) => red.id.into(),
        }
    }

    fn get_pool_ref(&self) -> Self::TPoolId {
        match self {
            ClassicalAMMOrder::Swap(swap) => swap.pool_id.0 .0,
            ClassicalAMMOrder::Deposit(dep) => dep.pool_id.0 .0,
            ClassicalAMMOrder::Redeem(red) => red.pool_id.0 .0,
        }
    }
}

impl<Ctx> TryFromLedger<BabbageTransactionOutput, Ctx> for ClassicalAMMOrder
where
    Ctx: Has<OutputRef>,
{
    fn try_from_ledger(repr: &BabbageTransactionOutput, ctx: Ctx) -> Option<Self> {
        ClassicalOnChainLimitSwap::try_from_ledger(repr, ctx.get())
            .map(|swap| ClassicalAMMOrder::Swap(swap))
            .or_else(|| {
                ClassicalOnChainDeposit::try_from_ledger(repr, ctx.get())
                    .map(|deposit| ClassicalAMMOrder::Deposit(deposit))
            })
            .or_else(|| {
                ClassicalOnChainRedeem::try_from_ledger(repr, ctx.get())
                    .map(|redeem| ClassicalAMMOrder::Redeem(redeem))
            })
    }
}

pub struct RunClassicalAMMOrderOrderOverPool<Pool>(pub Bundled<Pool, FinalizedTxOut>);

impl<Ctx> RunOrder<Bundled<ClassicalAMMOrder, FinalizedTxOut>, Ctx, SignedTxBuilder>
    for RunClassicalAMMOrderOrderOverPool<CFMMPool>
where
    Ctx: Clone
        + Has<Collateral>
        + Has<NetworkId>
        + Has<OperatorRewardAddress>
        + Has<DeployedValidator<{ ConstFnPoolV1 as u8 }>>
        + Has<DeployedValidator<{ ConstFnPoolV2 as u8 }>>
        + Has<DeployedValidator<{ ConstFnPoolSwap as u8 }>>
        + Has<DeployedValidator<{ ConstFnPoolDeposit as u8 }>>
        + Has<DeployedValidator<{ ConstFnPoolRedeem as u8 }>>,
{
    fn try_run(
        self,
        Bundled(order, ord_bearer): Bundled<ClassicalAMMOrder, FinalizedTxOut>,
        ctx: Ctx,
    ) -> Result<(SignedTxBuilder, Predicted<Self>), RunOrderError<Bundled<ClassicalAMMOrder, FinalizedTxOut>>>
    {
        let RunClassicalAMMOrderOrderOverPool(pool_bundle) = self;
        match order {
            ClassicalAMMOrder::Swap(swap) => RunAnyCFMMOrderOverPool(pool_bundle)
                .try_run(Bundled(swap, ord_bearer), ctx)
                .map(|(txb, res)| {
                    (
                        txb,
                        res.map(|wrapper| RunClassicalAMMOrderOrderOverPool(wrapper.0)),
                    )
                })
                .map_err(|err| {
                    err.map(|Bundled(swap, bundle)| Bundled(ClassicalAMMOrder::Swap(swap), bundle))
                }),
            ClassicalAMMOrder::Deposit(deposit) => RunAnyCFMMOrderOverPool(pool_bundle)
                .try_run(Bundled(deposit.clone(), ord_bearer), ctx)
                .map(|(txb, res)| {
                    (
                        txb,
                        res.map(|wrapper| RunClassicalAMMOrderOrderOverPool(wrapper.0)),
                    )
                })
                .map_err(|err| {
                    err.map(|Bundled(swap, bundle)| Bundled(ClassicalAMMOrder::Deposit(deposit), bundle))
                }),
            ClassicalAMMOrder::Redeem(redeem) => RunAnyCFMMOrderOverPool(pool_bundle)
                .try_run(Bundled(redeem.clone(), ord_bearer), ctx)
                .map(|(txb, res)| {
                    (
                        txb,
                        res.map(|wrapper| RunClassicalAMMOrderOrderOverPool(wrapper.0)),
                    )
                })
                .map_err(|err| {
                    err.map(|Bundled(swap, bundle)| Bundled(ClassicalAMMOrder::Redeem(redeem), bundle))
                }),
        }
    }
}
