use bloom_offchain::execution_engine::bundled::Bundled;
use cml_chain::builders::tx_builder::SignedTxBuilder;

use spectrum_cardano_lib::collateral::Collateral;
use spectrum_cardano_lib::output::FinalizedTxOut;
use spectrum_cardano_lib::NetworkId;
use spectrum_offchain::data::event::Predicted;
use spectrum_offchain::data::Has;
use spectrum_offchain::executor::{RunOrder, RunOrderError};
use spectrum_offchain_cardano::creds::OperatorRewardAddress;
use spectrum_offchain_cardano::data::balance_order::RunBalanceAMMOrderOverPool;

use spectrum_offchain_cardano::data::order::{ClassicalAMMOrder, RunClassicalAMMOrderOverPool};

use spectrum_offchain_cardano::data::pool::AnyPool;
use spectrum_offchain_cardano::data::pool::AnyPool::{BalancedCFMM, PureCFMM, StableCFMM};
use spectrum_offchain_cardano::data::stable_order::RunStableAMMOrderOverPool;
use spectrum_offchain_cardano::deployment::DeployedValidator;
use spectrum_offchain_cardano::deployment::ProtocolValidator::{BalanceFnPoolDeposit, BalanceFnPoolRedeem, BalanceFnPoolV1, ConstFnFeeSwitchPoolDeposit, ConstFnFeeSwitchPoolRedeem, ConstFnFeeSwitchPoolSwap, ConstFnPoolDeposit, ConstFnPoolFeeSwitch, ConstFnPoolFeeSwitchBiDirFee, ConstFnPoolRedeem, ConstFnPoolSwap, ConstFnPoolV1, ConstFnPoolV2, StableFnPoolT2T, StableFnPoolT2TDeposit, StableFnPoolT2TRedeem};

/// Magnet for local instances.
#[repr(transparent)]
pub struct PoolMagnet<T>(pub T);

impl<Ctx> RunOrder<Bundled<ClassicalAMMOrder, FinalizedTxOut>, Ctx, SignedTxBuilder>
    for PoolMagnet<Bundled<AnyPool, FinalizedTxOut>>
where
    Ctx: Clone
        + Has<NetworkId>
        + Has<Collateral>
        + Has<OperatorRewardAddress>
        + Has<DeployedValidator<{ ConstFnPoolV1 as u8 }>>
        + Has<DeployedValidator<{ ConstFnPoolV2 as u8 }>>
        + Has<DeployedValidator<{ ConstFnPoolFeeSwitch as u8 }>>
        + Has<DeployedValidator<{ ConstFnPoolFeeSwitchBiDirFee as u8 }>>
        + Has<DeployedValidator<{ ConstFnPoolSwap as u8 }>>
        + Has<DeployedValidator<{ ConstFnPoolDeposit as u8 }>>
        + Has<DeployedValidator<{ ConstFnPoolRedeem as u8 }>>
        + Has<DeployedValidator<{ ConstFnFeeSwitchPoolSwap as u8 }>>
        + Has<DeployedValidator<{ ConstFnFeeSwitchPoolDeposit as u8 }>>
        + Has<DeployedValidator<{ ConstFnFeeSwitchPoolRedeem as u8 }>>
        + Has<DeployedValidator<{ BalanceFnPoolV1 as u8 }>>
        + Has<DeployedValidator<{ BalanceFnPoolDeposit as u8 }>>
        + Has<DeployedValidator<{ BalanceFnPoolRedeem as u8 }>>
        + Has<DeployedValidator<{ StableFnPoolT2T as u8 }>>
        + Has<DeployedValidator<{ StableFnPoolT2TDeposit as u8 }>>
        + Has<DeployedValidator<{ StableFnPoolT2TRedeem as u8 }>>,
{
    fn try_run(
        self,
        order: Bundled<ClassicalAMMOrder, FinalizedTxOut>,
        ctx: Ctx,
    ) -> Result<(SignedTxBuilder, Predicted<Self>), RunOrderError<Bundled<ClassicalAMMOrder, FinalizedTxOut>>>
    {
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
