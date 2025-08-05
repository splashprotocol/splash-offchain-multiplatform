use cml_chain::builders::tx_builder::SignedTxBuilder;

use bloom_offchain::execution_engine::bundled::Bundled;
use spectrum_cardano_lib::collateral::Collateral;
use spectrum_cardano_lib::output::FinalizedTxOut;
use spectrum_cardano_lib::NetworkId;
use spectrum_offchain::domain::event::Predicted;
use spectrum_offchain::domain::Has;
use spectrum_offchain::executor::{RunOrder, RunOrderError};
use spectrum_offchain_cardano::creds::OperatorRewardAddress;
use spectrum_offchain_cardano::data::balance_order::RunBalanceAMMOrderOverPool;
use spectrum_offchain_cardano::data::dao_request::DAOContext;
use spectrum_offchain_cardano::data::order::{Order, RunClassicalAMMOrderOverPool};
use spectrum_offchain_cardano::data::pool::AnyPool;
use spectrum_offchain_cardano::data::pool::AnyPool::{BalancedCFMM, PureCFMM, StableCFMM};
use spectrum_offchain_cardano::data::quadratic_pool::QuadraticPool;
use spectrum_offchain_cardano::data::royalty_withdraw_request::RoyaltyWithdrawContext;
use spectrum_offchain_cardano::data::stable_order::RunStableAMMOrderOverPool;
use spectrum_offchain_cardano::deployment::DeployedValidator;
use spectrum_offchain_cardano::deployment::ProtocolValidator::{
    BalanceFnPoolDeposit, BalanceFnPoolRedeem, BalanceFnPoolV1, BalanceFnPoolV2, ConstFnFeeSwitchPoolDeposit,
    ConstFnFeeSwitchPoolRedeem, ConstFnFeeSwitchPoolSwap, ConstFnPoolDeposit, ConstFnPoolFeeSwitch,
    ConstFnPoolFeeSwitchBiDirFee, ConstFnPoolFeeSwitchV2, ConstFnPoolRedeem, ConstFnPoolSwap, ConstFnPoolV1,
    ConstFnPoolV2, RoyaltyPoolDAOV1, RoyaltyPoolDAOV1Request, RoyaltyPoolRoyaltyWithdraw,
    RoyaltyPoolRoyaltyWithdrawLedgerFixed, RoyaltyPoolRoyaltyWithdrawV2, RoyaltyPoolV1, RoyaltyPoolV1Deposit,
    RoyaltyPoolV1LedgerFixed, RoyaltyPoolV1Redeem, RoyaltyPoolV1RoyaltyWithdrawRequest, RoyaltyPoolV2,
    RoyaltyPoolV2DAO, RoyaltyPoolV2Deposit, RoyaltyPoolV2Redeem, RoyaltyPoolV2RoyaltyWithdrawRequest,
    StableFnPoolT2T, StableFnPoolT2TDeposit, StableFnPoolT2TRedeem,
};

/// Magnet for local instances.
#[repr(transparent)]
pub struct PoolMagnet<T>(pub T);

impl<Ctx> RunOrder<Bundled<Order, FinalizedTxOut>, Ctx, SignedTxBuilder>
    for PoolMagnet<Bundled<AnyPool, FinalizedTxOut>>
where
    Ctx: Clone
        + Has<NetworkId>
        + Has<Collateral>
        + Has<OperatorRewardAddress>
        + Has<DAOContext>
        + Has<RoyaltyWithdrawContext>
        + Has<DeployedValidator<{ ConstFnPoolV1 as u8 }>>
        + Has<DeployedValidator<{ ConstFnPoolV2 as u8 }>>
        + Has<DeployedValidator<{ ConstFnPoolFeeSwitch as u8 }>>
        + Has<DeployedValidator<{ ConstFnPoolFeeSwitchV2 as u8 }>>
        + Has<DeployedValidator<{ ConstFnPoolFeeSwitchBiDirFee as u8 }>>
        + Has<DeployedValidator<{ ConstFnPoolSwap as u8 }>>
        + Has<DeployedValidator<{ ConstFnPoolDeposit as u8 }>>
        + Has<DeployedValidator<{ ConstFnPoolRedeem as u8 }>>
        + Has<DeployedValidator<{ ConstFnFeeSwitchPoolSwap as u8 }>>
        + Has<DeployedValidator<{ ConstFnFeeSwitchPoolDeposit as u8 }>>
        + Has<DeployedValidator<{ ConstFnFeeSwitchPoolRedeem as u8 }>>
        + Has<DeployedValidator<{ BalanceFnPoolV1 as u8 }>>
        + Has<DeployedValidator<{ BalanceFnPoolV2 as u8 }>>
        + Has<DeployedValidator<{ BalanceFnPoolDeposit as u8 }>>
        + Has<DeployedValidator<{ BalanceFnPoolRedeem as u8 }>>
        + Has<DeployedValidator<{ StableFnPoolT2T as u8 }>>
        + Has<DeployedValidator<{ StableFnPoolT2TDeposit as u8 }>>
        + Has<DeployedValidator<{ StableFnPoolT2TRedeem as u8 }>>
        + Has<DeployedValidator<{ RoyaltyPoolV1 as u8 }>>
        + Has<DeployedValidator<{ RoyaltyPoolV1LedgerFixed as u8 }>>
        + Has<DeployedValidator<{ RoyaltyPoolV2 as u8 }>>
        + Has<DeployedValidator<{ RoyaltyPoolV1Deposit as u8 }>>
        + Has<DeployedValidator<{ RoyaltyPoolV2Deposit as u8 }>>
        + Has<DeployedValidator<{ RoyaltyPoolV1Redeem as u8 }>>
        + Has<DeployedValidator<{ RoyaltyPoolV2Redeem as u8 }>>
        + Has<DeployedValidator<{ RoyaltyPoolV1RoyaltyWithdrawRequest as u8 }>>
        + Has<DeployedValidator<{ RoyaltyPoolV2RoyaltyWithdrawRequest as u8 }>>
        + Has<DeployedValidator<{ RoyaltyPoolRoyaltyWithdraw as u8 }>>
        + Has<DeployedValidator<{ RoyaltyPoolRoyaltyWithdrawLedgerFixed as u8 }>>
        + Has<DeployedValidator<{ RoyaltyPoolRoyaltyWithdrawV2 as u8 }>>
        + Has<DeployedValidator<{ RoyaltyPoolDAOV1Request as u8 }>>
        + Has<DeployedValidator<{ RoyaltyPoolDAOV1 as u8 }>>
        + Has<DeployedValidator<{ RoyaltyPoolV2DAO as u8 }>>,
{
    fn try_run(
        self,
        order: Bundled<Order, FinalizedTxOut>,
        ctx: Ctx,
    ) -> Result<(SignedTxBuilder, Predicted<Self>), RunOrderError<Bundled<Order, FinalizedTxOut>>> {
        let PoolMagnet(Bundled(pool, bearer)) = self;
        match pool {
            PureCFMM(cfmm_pool) => RunClassicalAMMOrderOverPool(Bundled(cfmm_pool, bearer))
                .try_run(order, ctx)
                .map(|(txb, Predicted(bundle))| (txb, Predicted(PoolMagnet(bundle.0.map(PureCFMM))))),
            BalancedCFMM(balance_pool) => RunBalanceAMMOrderOverPool(Bundled(balance_pool, bearer))
                .try_run(order, ctx)
                .map(|(txb, Predicted(bundle))| (txb, Predicted(PoolMagnet(bundle.0.map(BalancedCFMM))))),
            StableCFMM(stable_pool) => RunStableAMMOrderOverPool(Bundled(stable_pool, bearer))
                .try_run(order, ctx)
                .map(|(txb, Predicted(bundle))| (txb, Predicted(PoolMagnet(bundle.0.map(StableCFMM))))),
        }
    }
}

impl<Ctx> RunOrder<Bundled<Order, FinalizedTxOut>, Ctx, SignedTxBuilder>
    for PoolMagnet<Bundled<QuadraticPool, FinalizedTxOut>>
{
    fn try_run(
        self,
        _: Bundled<Order, FinalizedTxOut>,
        _: Ctx,
    ) -> Result<(SignedTxBuilder, Predicted<Self>), RunOrderError<Bundled<Order, FinalizedTxOut>>> {
        unreachable!()
    }
}
