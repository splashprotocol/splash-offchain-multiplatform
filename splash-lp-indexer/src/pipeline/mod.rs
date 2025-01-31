use crate::db::accounts::Accounts;
use crate::db::event_log::EventLog;
use crate::pipeline::log_events::log_lp_events;
use crate::pipeline::read_events::read_events;
use cardano_chain_sync::atomic_flow::{BlockEvents, TransactionHandle};
use cml_chain::transaction::{Transaction, TransactionOutput};
use cml_core::Slot;
use cml_crypto::ScriptHash;
use cml_multi_era::babbage::BabbageTransaction;
use either::Either;
use futures::{Stream, StreamExt};
use spectrum_cardano_lib::OutputRef;
use spectrum_offchain::domain::Has;
use spectrum_offchain::persistent_index::PersistentIndex;
use spectrum_offchain_cardano::data::pool::PoolValidation;
use spectrum_offchain_cardano::deployment::DeployedScriptInfo;
use spectrum_offchain_cardano::deployment::ProtocolValidator::*;
use std::collections::HashSet;

mod log_events;
pub mod read_events;

pub async fn log_events<U, Log, Cx, Index>(
    upstream: U,
    log: Log,
    context: Cx,
    index: Index,
    utxo_filter: HashSet<ScriptHash>,
) where
    U: Stream<
        Item = (
            BlockEvents<Either<BabbageTransaction, Transaction>>,
            TransactionHandle,
        ),
    >,
    Log: EventLog,
    Index: PersistentIndex<OutputRef, TransactionOutput>,
    Cx: Has<DeployedScriptInfo<{ ConstFnPoolV1 as u8 }>>
        + Has<DeployedScriptInfo<{ ConstFnPoolV2 as u8 }>>
        + Has<DeployedScriptInfo<{ ConstFnPoolFeeSwitch as u8 }>>
        + Has<DeployedScriptInfo<{ ConstFnPoolFeeSwitchV2 as u8 }>>
        + Has<DeployedScriptInfo<{ ConstFnPoolFeeSwitchBiDirFee as u8 }>>
        + Has<DeployedScriptInfo<{ BalanceFnPoolV1 as u8 }>>
        + Has<DeployedScriptInfo<{ BalanceFnPoolV2 as u8 }>>
        + Has<DeployedScriptInfo<{ StableFnPoolT2T as u8 }>>
        + Has<DeployedScriptInfo<{ RoyaltyPoolV1 as u8 }>>
        + Has<PoolValidation>,
{
    log_lp_events(
        upstream.then(|(block, tx_handle)| {
            let lp_events = read_events(block, &context, &index, &utxo_filter);
            async move { (lp_events.await, tx_handle) }
        }),
        &log,
    )
    .await
}

pub async fn update_accounts<DB: Accounts>(db: DB, genesis_slot: Slot, confirmation_delay_blocks: u64) {
    loop {
        if !db
            .try_update_accounts(genesis_slot, confirmation_delay_blocks)
            .await
        {
            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
        }
    }
}
