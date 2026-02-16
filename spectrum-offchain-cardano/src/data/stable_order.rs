use crate::creds::OperatorRewardAddress;
use crate::data::order::Order;
use crate::data::pool::try_run_order_against_pool;
use crate::data::stable_pool_t2t::StablePoolT2T;
use crate::deployment::DeployedValidator;
use crate::deployment::ProtocolValidator::{
    BalanceFnPoolDeposit, BalanceFnPoolRedeem, BalanceFnPoolV1, ConstFnFeeSwitchPoolDeposit,
    ConstFnFeeSwitchPoolRedeem, ConstFnFeeSwitchPoolSwap, ConstFnPoolDeposit, ConstFnPoolRedeem,
    ConstFnPoolSwap, ConstFnPoolV1, ConstFnPoolV2, RoyaltyPoolV1, RoyaltyPoolV1Deposit, RoyaltyPoolV1Redeem,
    RoyaltyPoolV1RoyaltyWithdrawRequest, RoyaltyPoolV2, RoyaltyPoolV2Deposit, RoyaltyPoolV2Redeem,
    StableFnPoolT2T, StableFnPoolT2TDeposit, StableFnPoolT2TRedeem,
};
use bloom_offchain::execution_engine::bundled::Bundled;
use cml_chain::builders::tx_builder::SignedTxBuilder;
use spectrum_cardano_lib::collateral::Collateral;
use spectrum_cardano_lib::output::FinalizedTxOut;
use spectrum_cardano_lib::NetworkId;
use spectrum_offchain::domain::event::Predicted;
use spectrum_offchain::domain::Has;
use spectrum_offchain::executor::RunOrderError::Fatal;
use spectrum_offchain::executor::{RunOrder, RunOrderError};

pub struct RunStableAMMOrderOverPool<Pool>(pub Bundled<Pool, FinalizedTxOut>);

impl<Ctx> RunOrder<Bundled<Order, FinalizedTxOut>, Ctx, SignedTxBuilder>
    for RunStableAMMOrderOverPool<StablePoolT2T>
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
        + Has<DeployedValidator<{ ConstFnPoolRedeem as u8 }>>
        + Has<DeployedValidator<{ ConstFnFeeSwitchPoolSwap as u8 }>>
        + Has<DeployedValidator<{ ConstFnFeeSwitchPoolDeposit as u8 }>>
        + Has<DeployedValidator<{ ConstFnFeeSwitchPoolRedeem as u8 }>>
        + Has<DeployedValidator<{ StableFnPoolT2T as u8 }>>
        + Has<DeployedValidator<{ StableFnPoolT2TDeposit as u8 }>>
        + Has<DeployedValidator<{ StableFnPoolT2TRedeem as u8 }>>
        + Has<DeployedValidator<{ RoyaltyPoolV1 as u8 }>>
        + Has<DeployedValidator<{ RoyaltyPoolV2 as u8 }>>
        + Has<DeployedValidator<{ RoyaltyPoolV1Deposit as u8 }>>
        + Has<DeployedValidator<{ RoyaltyPoolV2Deposit as u8 }>>
        + Has<DeployedValidator<{ RoyaltyPoolV1Redeem as u8 }>>
        + Has<DeployedValidator<{ RoyaltyPoolV2Redeem as u8 }>>
        + Has<DeployedValidator<{ RoyaltyPoolV1RoyaltyWithdrawRequest as u8 }>>,
{
    fn try_run(
        self,
        Bundled(order, ord_bearer): Bundled<Order, FinalizedTxOut>,
        ctx: Ctx,
    ) -> Result<(SignedTxBuilder, Predicted<Self>), RunOrderError<Bundled<Order, FinalizedTxOut>>> {
        let RunStableAMMOrderOverPool(pool_bundle) = self;
        match order {
            Order::Deposit(deposit) => {
                try_run_order_against_pool(pool_bundle, Bundled(deposit.clone(), ord_bearer), ctx)
                    .map(|(txb, res)| (txb, res.map(RunStableAMMOrderOverPool)))
                    .map_err(|err| err.map(|Bundled(_swap, bundle)| Bundled(Order::Deposit(deposit), bundle)))
            }
            Order::Redeem(redeem) => {
                try_run_order_against_pool(pool_bundle, Bundled(redeem.clone(), ord_bearer), ctx)
                    .map(|(txb, res)| (txb, res.map(RunStableAMMOrderOverPool)))
                    .map_err(|err| err.map(|Bundled(_swap, bundle)| Bundled(Order::Redeem(redeem), bundle)))
            }
            _ => Err(Fatal(
                format!("Unsupported order operation {}", ord_bearer.1),
                Bundled(order, ord_bearer),
            )),
        }
    }
}
