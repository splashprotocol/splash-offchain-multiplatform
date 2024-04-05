use crate::creds::OperatorRewardAddress;
use crate::data::balance_pool::BalancePool;

use crate::data::deposit::ClassicalOnChainDeposit;

use crate::data::pool::try_run_order_against_pool;
use crate::data::redeem::ClassicalOnChainRedeem;
use crate::deployment::ProtocolValidator::{
    BalanceFnPoolDeposit, BalanceFnPoolRedeem, BalanceFnPoolV1, ConstFnPoolDeposit, ConstFnPoolRedeem,
    ConstFnPoolSwap, ConstFnPoolV1, ConstFnPoolV2,
};
use crate::deployment::{DeployedScriptInfo, DeployedValidator};
use bloom_offchain::execution_engine::bundled::Bundled;
use cml_chain::builders::tx_builder::SignedTxBuilder;
use cml_crypto::ScriptHash;
use cml_multi_era::babbage::BabbageTransactionOutput;
use spectrum_cardano_lib::collateral::Collateral;
use spectrum_cardano_lib::output::FinalizedTxOut;
use spectrum_cardano_lib::{NetworkId, OutputRef};
use spectrum_offchain::backlog::data::{OrderWeight, Weighted};
use spectrum_offchain::data::order::{SpecializedOrder, UniqueOrder};
use spectrum_offchain::data::unique_entity::Predicted;
use spectrum_offchain::data::Has;
use spectrum_offchain::executor::{RunOrder, RunOrderError};
use spectrum_offchain::ledger::TryFromLedger;
use std::fmt::{Display, Formatter};
use std::hash::{Hash, Hasher};

pub enum BalanceAMMOrder {
    Deposit(ClassicalOnChainDeposit),
    Redeem(ClassicalOnChainRedeem),
}

impl Display for BalanceAMMOrder {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str("BalanceAMMOrder")
    }
}

impl Weighted for BalanceAMMOrder {
    fn weight(&self) -> OrderWeight {
        match self {
            BalanceAMMOrder::Deposit(deposit) => OrderWeight::from(deposit.order.ex_fee),
            BalanceAMMOrder::Redeem(redeem) => OrderWeight::from(redeem.order.ex_fee),
        }
    }
}

impl PartialEq for BalanceAMMOrder {
    fn eq(&self, other: &Self) -> bool {
        <Self as UniqueOrder>::get_self_ref(self).eq(&<Self as UniqueOrder>::get_self_ref(other))
    }
}

impl Eq for BalanceAMMOrder {}

impl Hash for BalanceAMMOrder {
    fn hash<H: Hasher>(&self, state: &mut H) {
        <Self as UniqueOrder>::get_self_ref(self).hash(state)
    }
}

impl SpecializedOrder for BalanceAMMOrder {
    type TOrderId = OutputRef;
    type TPoolId = ScriptHash;

    fn get_self_ref(&self) -> Self::TOrderId {
        match self {
            BalanceAMMOrder::Deposit(dep) => dep.id.into(),
            BalanceAMMOrder::Redeem(red) => red.id.into(),
        }
    }

    fn get_pool_ref(&self) -> Self::TPoolId {
        match self {
            BalanceAMMOrder::Deposit(dep) => dep.pool_id.0 .0,
            BalanceAMMOrder::Redeem(red) => red.pool_id.0 .0,
        }
    }
}

impl<Ctx> TryFromLedger<BabbageTransactionOutput, Ctx> for BalanceAMMOrder
where
    Ctx: Has<OutputRef>
        + Has<DeployedScriptInfo<{ ConstFnPoolDeposit as u8 }>>
        + Has<DeployedScriptInfo<{ ConstFnPoolRedeem as u8 }>>,
{
    fn try_from_ledger(repr: &BabbageTransactionOutput, ctx: &Ctx) -> Option<Self> {
        ClassicalOnChainDeposit::try_from_ledger(repr, ctx)
            .map(|deposit| BalanceAMMOrder::Deposit(deposit))
            .or_else(|| {
                ClassicalOnChainRedeem::try_from_ledger(repr, ctx)
                    .map(|redeem| BalanceAMMOrder::Redeem(redeem))
            })
    }
}

pub struct RunBalanceAMMOrderOverPool<Pool>(pub Bundled<Pool, FinalizedTxOut>);

impl<Ctx> RunOrder<Bundled<BalanceAMMOrder, FinalizedTxOut>, Ctx, SignedTxBuilder>
    for RunBalanceAMMOrderOverPool<BalancePool>
where
    Ctx: Clone
        + Has<Collateral>
        + Has<NetworkId>
        + Has<OperatorRewardAddress>
        + Has<DeployedValidator<{ BalanceFnPoolV1 as u8 }>>
        + Has<DeployedValidator<{ BalanceFnPoolDeposit as u8 }>>
        + Has<DeployedValidator<{ BalanceFnPoolRedeem as u8 }>>
        // comes from common execution for deposit and redeem for balance pool. todo: cleanup
        + Has<DeployedValidator<{ ConstFnPoolV1 as u8 }>>
        + Has<DeployedValidator<{ ConstFnPoolV2 as u8 }>>
        + Has<DeployedValidator<{ ConstFnPoolSwap as u8 }>>
        + Has<DeployedValidator<{ ConstFnPoolDeposit as u8 }>>
        + Has<DeployedValidator<{ ConstFnPoolRedeem as u8 }>>,
{
    fn try_run(
        self,
        Bundled(order, ord_bearer): Bundled<BalanceAMMOrder, FinalizedTxOut>,
        ctx: Ctx,
    ) -> Result<(SignedTxBuilder, Predicted<Self>), RunOrderError<Bundled<BalanceAMMOrder, FinalizedTxOut>>>
    {
        let RunBalanceAMMOrderOverPool(pool_bundle) = self;
        match order {
            BalanceAMMOrder::Deposit(deposit) => {
                try_run_order_against_pool(pool_bundle, Bundled(deposit.clone(), ord_bearer), ctx)
                    .map(|(txb, res)| (txb, res.map(RunBalanceAMMOrderOverPool)))
                    .map_err(|err| {
                        err.map(|Bundled(_swap, bundle)| Bundled(BalanceAMMOrder::Deposit(deposit), bundle))
                    })
            }
            BalanceAMMOrder::Redeem(redeem) => {
                try_run_order_against_pool(pool_bundle, Bundled(redeem.clone(), ord_bearer), ctx)
                    .map(|(txb, res)| (txb, res.map(RunBalanceAMMOrderOverPool)))
                    .map_err(|err| {
                        err.map(|Bundled(_swap, bundle)| Bundled(BalanceAMMOrder::Redeem(redeem), bundle))
                    })
            }
        }
    }
}
