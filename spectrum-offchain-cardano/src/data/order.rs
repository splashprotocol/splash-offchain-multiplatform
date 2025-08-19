use std::fmt::{Display, Formatter};
use std::hash::{Hash, Hasher};

use cml_chain::builders::tx_builder::SignedTxBuilder;
use cml_chain::plutus::{ConstrPlutusData, PlutusData};
use cml_chain::transaction::TransactionOutput;
use cml_chain::utils::BigInteger;

use bloom_offchain::execution_engine::bundled::Bundled;
use spectrum_cardano_lib::collateral::Collateral;
use spectrum_cardano_lib::output::FinalizedTxOut;

use spectrum_offchain::backlog::data::{OrderWeight, Weighted};
use spectrum_offchain::domain::event::Predicted;
use spectrum_offchain::domain::order::{SpecializedOrder, UniqueOrder};
use spectrum_offchain::domain::Has;
use spectrum_offchain::executor::{RunOrder, RunOrderError};
use spectrum_offchain::ledger::TryFromLedger;

use crate::creds::OperatorRewardAddress;

use crate::data::cfmm_pool::ConstFnPool;
use crate::data::dao_request::{DAOContext, DAOV1ActionOrderValidation, OnChainDAOActionRequest};
use crate::data::deposit::{ClassicalOnChainDeposit, DepositOrderValidation};
use crate::data::pool::try_run_order_against_pool;
use crate::data::redeem::{ClassicalOnChainRedeem, RedeemOrderValidation};
use crate::data::royalty_withdraw_request::{
    OnChainRoyaltyWithdraw, RoyaltyWithdrawContext, RoyaltyWithdrawOrderValidation,
};
use crate::data::PoolId;
use crate::deployment::ProtocolValidator::{
    BalanceFnPoolDeposit, BalanceFnPoolRedeem, BalanceFnPoolV1, BalanceFnPoolV2, ConstFnFeeSwitchPoolDeposit,
    ConstFnFeeSwitchPoolRedeem, ConstFnFeeSwitchPoolSwap, ConstFnPoolDeposit, ConstFnPoolFeeSwitch,
    ConstFnPoolFeeSwitchBiDirFee, ConstFnPoolFeeSwitchV2, ConstFnPoolRedeem, ConstFnPoolSwap, ConstFnPoolV1,
    ConstFnPoolV2, RoyaltyPoolDAOV1, RoyaltyPoolDAOV1Request, RoyaltyPoolRoyaltyWithdraw,
    RoyaltyPoolRoyaltyWithdrawLedgerFixed, RoyaltyPoolRoyaltyWithdrawV2, RoyaltyPoolV1, RoyaltyPoolV1Deposit,
    RoyaltyPoolV1LedgerFixed, RoyaltyPoolV1Redeem, RoyaltyPoolV1RoyaltyWithdrawRequest, RoyaltyPoolV2,
    RoyaltyPoolV2DAO, RoyaltyPoolV2Deposit, RoyaltyPoolV2Redeem, RoyaltyPoolV2RoyaltyWithdrawRequest,
    StableFnPoolT2T, StableFnPoolT2TDeposit, StableFnPoolT2TRedeem,
};
use crate::deployment::{DeployedScriptInfo, DeployedValidator};
use spectrum_cardano_lib::{NetworkId, OutputRef, Token};

pub struct Base;

pub struct Quote;

pub struct PoolNft;

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct ClassicalOrder<Id, Ord> {
    pub id: Id,
    pub pool_id: PoolId,
    pub order: Ord,
}

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub enum OrderType {
    BalanceFn,
    ConstFnFeeSwitch,
    ConstFn,
    StableFn,
    RoyaltyConstFnV1,
    RoyaltyConstFnV2,
}

impl<Id: Clone, Ord> Has<Id> for ClassicalOrder<Id, Ord> {
    fn select<U: type_equalities::IsEqual<Id>>(&self) -> Id {
        self.id.clone()
    }
}

pub enum ClassicalOrderAction {
    Apply,
}

impl ClassicalOrderAction {
    pub fn to_plutus_data(self) -> PlutusData {
        match self {
            ClassicalOrderAction::Apply => PlutusData::Integer(BigInteger::from(0)),
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
        let pool_in_ix_pd = PlutusData::Integer(BigInteger::from(self.pool_input_index));
        let order_in_ix_pd = PlutusData::Integer(BigInteger::from(self.order_input_index));
        let out_ix_pd = PlutusData::Integer(BigInteger::from(self.output_index));
        PlutusData::ConstrPlutusData(ConstrPlutusData::new(
            0,
            vec![pool_in_ix_pd, order_in_ix_pd, out_ix_pd, action_pd],
        ))
    }
}

#[derive(Debug, Clone)]
pub enum Order {
    Deposit(ClassicalOnChainDeposit),
    Redeem(ClassicalOnChainRedeem),
    RoyaltyWithdraw(OnChainRoyaltyWithdraw),
    DAOActionRequest(OnChainDAOActionRequest),
}

impl Display for Order {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str("ClassicalAMMOrder")
    }
}

impl Weighted for Order {
    fn weight(&self) -> OrderWeight {
        match self {
            Order::Deposit(deposit) => OrderWeight::from(deposit.order.ex_fee),
            Order::Redeem(redeem) => OrderWeight::from(redeem.order.ex_fee),
            Order::RoyaltyWithdraw(_) => OrderWeight::from(0),
            Order::DAOActionRequest(_) => OrderWeight::from(0),
        }
    }
}

impl PartialEq for Order {
    fn eq(&self, other: &Self) -> bool {
        <Self as UniqueOrder>::get_self_ref(self).eq(&<Self as UniqueOrder>::get_self_ref(other))
    }
}

impl Eq for Order {}

impl Hash for Order {
    fn hash<H: Hasher>(&self, state: &mut H) {
        <Self as UniqueOrder>::get_self_ref(self).hash(state)
    }
}

impl SpecializedOrder for Order {
    type TOrderId = OutputRef;
    type TPoolId = Token;

    fn get_self_ref(&self) -> Self::TOrderId {
        match self {
            Order::Deposit(dep) => dep.id.into(),
            Order::Redeem(red) => red.id.into(),
            Order::RoyaltyWithdraw(withdraw) => withdraw.id.into(),
            Order::DAOActionRequest(dao_request) => dao_request.id.into(),
        }
    }

    fn get_pool_ref(&self) -> Self::TPoolId {
        match self {
            Order::Deposit(dep) => dep.pool_id.into(),
            Order::Redeem(red) => red.pool_id.into(),
            Order::RoyaltyWithdraw(withdraw) => withdraw.pool_id.into(),
            Order::DAOActionRequest(dao_request) => dao_request.pool_id.into(),
        }
    }
}

impl<Ctx> TryFromLedger<TransactionOutput, Ctx> for Order
where
    Ctx: Has<OutputRef>
        + Has<DeployedScriptInfo<{ ConstFnFeeSwitchPoolSwap as u8 }>>
        + Has<DeployedScriptInfo<{ ConstFnFeeSwitchPoolDeposit as u8 }>>
        + Has<DeployedScriptInfo<{ ConstFnFeeSwitchPoolRedeem as u8 }>>
        + Has<DeployedScriptInfo<{ ConstFnPoolSwap as u8 }>>
        + Has<DeployedScriptInfo<{ ConstFnPoolDeposit as u8 }>>
        + Has<DeployedScriptInfo<{ ConstFnPoolRedeem as u8 }>>
        + Has<DeployedScriptInfo<{ BalanceFnPoolDeposit as u8 }>>
        + Has<DeployedScriptInfo<{ BalanceFnPoolRedeem as u8 }>>
        + Has<DeployedScriptInfo<{ StableFnPoolT2TDeposit as u8 }>>
        + Has<DeployedScriptInfo<{ StableFnPoolT2TRedeem as u8 }>>
        + Has<DeployedScriptInfo<{ RoyaltyPoolV1Deposit as u8 }>>
        + Has<DeployedScriptInfo<{ RoyaltyPoolV2Deposit as u8 }>>
        + Has<DeployedScriptInfo<{ RoyaltyPoolV1Redeem as u8 }>>
        + Has<DeployedScriptInfo<{ RoyaltyPoolV2Redeem as u8 }>>
        + Has<DeployedScriptInfo<{ RoyaltyPoolV1RoyaltyWithdrawRequest as u8 }>>
        + Has<DeployedScriptInfo<{ RoyaltyPoolV2RoyaltyWithdrawRequest as u8 }>>
        + Has<DeployedScriptInfo<{ RoyaltyPoolDAOV1Request as u8 }>>
        + Has<DepositOrderValidation>
        + Has<RedeemOrderValidation>
        + Has<RoyaltyWithdrawOrderValidation>
        + Has<DAOV1ActionOrderValidation>,
{
    fn try_from_ledger(repr: &TransactionOutput, ctx: &Ctx) -> Option<Self> {
        ClassicalOnChainDeposit::try_from_ledger(repr, ctx)
            .map(|deposit| Order::Deposit(deposit))
            .or_else(|| {
                ClassicalOnChainRedeem::try_from_ledger(repr, ctx).map(|redeem| Order::Redeem(redeem))
            })
            .or_else(|| {
                ClassicalOnChainRedeem::try_from_ledger(repr, ctx).map(|redeem| Order::Redeem(redeem))
            })
            .or_else(|| {
                OnChainRoyaltyWithdraw::try_from_ledger(repr, ctx)
                    .map(|royalty_withdraw| Order::RoyaltyWithdraw(royalty_withdraw))
            })
            .or_else(|| {
                OnChainDAOActionRequest::try_from_ledger(repr, ctx)
                    .map(|royalty_withdraw| Order::DAOActionRequest(royalty_withdraw))
            })
    }
}

pub struct RunClassicalAMMOrderOverPool<Pool>(pub Bundled<Pool, FinalizedTxOut>);

impl<Ctx> RunOrder<Bundled<Order, FinalizedTxOut>, Ctx, SignedTxBuilder>
    for RunClassicalAMMOrderOverPool<ConstFnPool>
where
    Ctx: Clone
        + Has<Collateral>
        + Has<NetworkId>
        + Has<OperatorRewardAddress>
        + Has<DAOContext>
        + Has<RoyaltyWithdrawContext>
        + Has<DeployedValidator<{ ConstFnPoolV1 as u8 }>>
        + Has<DeployedValidator<{ ConstFnPoolV2 as u8 }>>
        + Has<DeployedValidator<{ ConstFnFeeSwitchPoolSwap as u8 }>>
        + Has<DeployedValidator<{ ConstFnPoolFeeSwitch as u8 }>>
        + Has<DeployedValidator<{ ConstFnPoolFeeSwitchV2 as u8 }>>
        + Has<DeployedValidator<{ ConstFnPoolFeeSwitchBiDirFee as u8 }>>
        + Has<DeployedValidator<{ ConstFnPoolSwap as u8 }>>
        + Has<DeployedValidator<{ ConstFnPoolDeposit as u8 }>>
        + Has<DeployedValidator<{ ConstFnPoolRedeem as u8 }>>
        + Has<DeployedValidator<{ ConstFnFeeSwitchPoolSwap as u8 }>>
        + Has<DeployedValidator<{ ConstFnFeeSwitchPoolDeposit as u8 }>>
        + Has<DeployedValidator<{ ConstFnFeeSwitchPoolRedeem as u8 }>>
        + Has<DeployedValidator<{ RoyaltyPoolV1 as u8 }>>
        + Has<DeployedValidator<{ RoyaltyPoolV1LedgerFixed as u8 }>>
        + Has<DeployedValidator<{ RoyaltyPoolV2 as u8 }>>
        + Has<DeployedValidator<{ RoyaltyPoolV1Deposit as u8 }>>
        + Has<DeployedValidator<{ RoyaltyPoolV2Deposit as u8 }>>
        + Has<DeployedValidator<{ RoyaltyPoolV1Redeem as u8 }>>
        + Has<DeployedValidator<{ RoyaltyPoolV2Redeem as u8 }>>
        + Has<DeployedValidator<{ RoyaltyPoolV1RoyaltyWithdrawRequest as u8 }>>
        + Has<DeployedValidator<{ RoyaltyPoolV2RoyaltyWithdrawRequest as u8 }>>
        + Has<DeployedValidator<{ RoyaltyPoolDAOV1Request as u8 }>>
        + Has<DeployedValidator<{ RoyaltyPoolDAOV1 as u8 }>>
        + Has<DeployedValidator<{ RoyaltyPoolV2DAO as u8 }>>
        + Has<DeployedValidator<{ RoyaltyPoolRoyaltyWithdraw as u8 }>>
        + Has<DeployedValidator<{ RoyaltyPoolRoyaltyWithdrawLedgerFixed as u8 }>>
        + Has<DeployedValidator<{ RoyaltyPoolRoyaltyWithdrawV2 as u8 }>>
        // comes from common execution for deposit and redeem for balance pool
        + Has<DeployedValidator<{ BalanceFnPoolV1 as u8 }>>
        + Has<DeployedValidator<{ BalanceFnPoolV2 as u8 }>>
        + Has<DeployedValidator<{ BalanceFnPoolDeposit as u8 }>>
        + Has<DeployedValidator<{ BalanceFnPoolRedeem as u8 }>>
        + Has<DeployedValidator<{ StableFnPoolT2T as u8 }>>
        + Has<DeployedValidator<{ StableFnPoolT2TDeposit as u8 }>>
        + Has<DeployedValidator<{ StableFnPoolT2TRedeem as u8 }>>,
{
    fn try_run(
        self,
        Bundled(order, ord_bearer): Bundled<Order, FinalizedTxOut>,
        ctx: Ctx,
    ) -> Result<(SignedTxBuilder, Predicted<Self>), RunOrderError<Bundled<Order, FinalizedTxOut>>> {
        let RunClassicalAMMOrderOverPool(pool_bundle) = self;
        match order {
            Order::Deposit(deposit) => {
                try_run_order_against_pool(pool_bundle, Bundled(deposit.clone(), ord_bearer), ctx)
                    .map(|(txb, res)| (txb, res.map(RunClassicalAMMOrderOverPool)))
                    .map_err(|err| err.map(|Bundled(_swap, bundle)| Bundled(Order::Deposit(deposit), bundle)))
            }
            Order::Redeem(redeem) => {
                try_run_order_against_pool(pool_bundle, Bundled(redeem.clone(), ord_bearer), ctx)
                    .map(|(txb, res)| (txb, res.map(RunClassicalAMMOrderOverPool)))
                    .map_err(|err| err.map(|Bundled(_swap, bundle)| Bundled(Order::Redeem(redeem), bundle)))
            }
            Order::RoyaltyWithdraw(royalty_withdraw) => {
                try_run_order_against_pool(pool_bundle, Bundled(royalty_withdraw.clone(), ord_bearer), ctx)
                    .map(|(txb, res)| (txb, res.map(RunClassicalAMMOrderOverPool)))
                    .map_err(|err| {
                        err.map(|Bundled(_swap, bundle)| {
                            Bundled(Order::RoyaltyWithdraw(royalty_withdraw), bundle)
                        })
                    })
            }
            Order::DAOActionRequest(dao_request) => {
                try_run_order_against_pool(pool_bundle, Bundled(dao_request.clone(), ord_bearer), ctx)
                    .map(|(txb, res)| (txb, res.map(RunClassicalAMMOrderOverPool)))
                    .map_err(|err| {
                        err.map(|Bundled(_swap, bundle)| {
                            Bundled(Order::DAOActionRequest(dao_request), bundle)
                        })
                    })
            }
        }
    }
}
