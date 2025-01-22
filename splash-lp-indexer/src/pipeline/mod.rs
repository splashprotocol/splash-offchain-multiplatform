use crate::event_log::EventLog;
use crate::pipeline::log_events::log_lp_events;
use crate::pipeline::read_events::read_events;
use cardano_chain_sync::atomic_flow::{BlockEvent, TransactionHandle};
use cml_chain::transaction::TransactionOutput;
use cml_crypto::ScriptHash;
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
    log: &Log,
    context: &Cx,
    index: &Index,
    utxo_filter: &HashSet<ScriptHash>,
) where
    U: Stream<Item = (BlockEvent, TransactionHandle)>,
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
        upstream.then(|(block_event, tx_handle)| async move {
            let lp_events = read_events(block_event, context, index, utxo_filter).await;
            (lp_events, tx_handle)
        }),
        log,
    )
    .await
}
