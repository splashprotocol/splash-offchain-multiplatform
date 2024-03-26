use bloom_offchain::execution_engine::bundled::Bundled;
use cml_chain::builders::tx_builder::SignedTxBuilder;

use spectrum_cardano_lib::collateral::Collateral;
use spectrum_cardano_lib::output::FinalizedTxOut;
use spectrum_cardano_lib::NetworkId;
use spectrum_offchain::data::unique_entity::Predicted;
use spectrum_offchain::data::Has;
use spectrum_offchain::executor::{RunOrder, RunOrderError};
use spectrum_offchain_cardano::creds::OperatorRewardAddress;

use spectrum_offchain_cardano::data::order::{ClassicalAMMOrder, RunClassicalAMMOrderOverPool};

use spectrum_offchain_cardano::data::pool::AnyPool;
use spectrum_offchain_cardano::data::pool::AnyPool::{BalancedCFMM, PureCFMM};
use spectrum_offchain_cardano::deployment::DeployedValidator;
use spectrum_offchain_cardano::deployment::ProtocolValidator::{
    BalanceFnPoolV1, ConstFnPoolDeposit, ConstFnPoolRedeem, ConstFnPoolSwap, ConstFnPoolV1, ConstFnPoolV2,
};

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
        + Has<DeployedValidator<{ ConstFnPoolSwap as u8 }>>
        + Has<DeployedValidator<{ ConstFnPoolDeposit as u8 }>>
        + Has<DeployedValidator<{ ConstFnPoolRedeem as u8 }>>
        + Has<DeployedValidator<{ BalanceFnPoolV1 as u8 }>>,
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
            BalancedCFMM(_balance_pool) => unreachable!(),
        }
    }
}
