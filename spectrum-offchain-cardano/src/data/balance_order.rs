use crate::creds::OperatorRewardAddress;
use crate::data::balance_pool::BalancePool;

use crate::data::deposit::ClassicalOnChainDeposit;

use crate::data::order::ClassicalAMMOrder;
use crate::data::pool::try_run_order_against_pool;
use crate::data::redeem::ClassicalOnChainRedeem;
use crate::deployment::ProtocolValidator::{
    BalanceFnPoolDeposit, BalanceFnPoolRedeem, BalanceFnPoolV1, BalanceFnPoolV2, ConstFnFeeSwitchPoolDeposit,
    ConstFnFeeSwitchPoolRedeem, ConstFnFeeSwitchPoolSwap, ConstFnPoolDeposit, ConstFnPoolRedeem,
    ConstFnPoolSwap, ConstFnPoolV1, ConstFnPoolV2, StableFnPoolT2T, StableFnPoolT2TDeposit,
    StableFnPoolT2TRedeem,
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
use spectrum_offchain::data::event::Predicted;
use spectrum_offchain::data::order::{SpecializedOrder, UniqueOrder};
use spectrum_offchain::data::Has;
use spectrum_offchain::executor::{RunOrder, RunOrderError};
use spectrum_offchain::ledger::TryFromLedger;
use std::fmt::{Display, Formatter};
use std::hash::{Hash, Hasher};

pub struct RunBalanceAMMOrderOverPool<Pool>(pub Bundled<Pool, FinalizedTxOut>);

impl<Ctx> RunOrder<Bundled<ClassicalAMMOrder, FinalizedTxOut>, Ctx, SignedTxBuilder>
    for RunBalanceAMMOrderOverPool<BalancePool>
where
    Ctx: Clone
        + Has<Collateral>
        + Has<NetworkId>
        + Has<OperatorRewardAddress>
        + Has<DeployedValidator<{ BalanceFnPoolV1 as u8 }>>
        + Has<DeployedValidator<{ BalanceFnPoolV2 as u8 }>>
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
        + Has<DeployedValidator<{ StableFnPoolT2TRedeem as u8 }>>,
{
    fn try_run(
        self,
        Bundled(order, ord_bearer): Bundled<ClassicalAMMOrder, FinalizedTxOut>,
        ctx: Ctx,
    ) -> Result<(SignedTxBuilder, Predicted<Self>), RunOrderError<Bundled<ClassicalAMMOrder, FinalizedTxOut>>>
    {
        let RunBalanceAMMOrderOverPool(pool_bundle) = self;
        match order {
            ClassicalAMMOrder::Deposit(deposit) => {
                try_run_order_against_pool(pool_bundle, Bundled(deposit.clone(), ord_bearer), ctx)
                    .map(|(txb, res)| (txb, res.map(RunBalanceAMMOrderOverPool)))
                    .map_err(|err| {
                        err.map(|Bundled(_swap, bundle)| Bundled(ClassicalAMMOrder::Deposit(deposit), bundle))
                    })
            }
            ClassicalAMMOrder::Redeem(redeem) => {
                try_run_order_against_pool(pool_bundle, Bundled(redeem.clone(), ord_bearer), ctx)
                    .map(|(txb, res)| (txb, res.map(RunBalanceAMMOrderOverPool)))
                    .map_err(|err| {
                        err.map(|Bundled(_swap, bundle)| Bundled(ClassicalAMMOrder::Redeem(redeem), bundle))
                    })
            }
            ClassicalAMMOrder::Swap(_) => unreachable!(),
        }
    }
}
