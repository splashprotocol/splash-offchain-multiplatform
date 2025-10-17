use crate::pipeline::log_events::log_onchain_events;
use crate::pipeline::resolve_gauges::translate_events;
use crate::position_db::accounts::Accounts;
use crate::position_db::event_log::EventLog;
use crate::position_db::mature_events::MatureEvents;
use crate::ve_index::VoteEscrowIndex;
use cardano_chain_sync::atomic_flow::{BlockEvents, TransactionHandle};
use cml_chain::transaction::Transaction;
use cml_crypto::ScriptHash;
use cml_multi_era::babbage::BabbageTransaction;
use either::Either;
use futures::FutureExt;
use futures::{Stream, StreamExt};
use spectrum_cardano_lib::tx_view::TimedOutput;
use spectrum_cardano_lib::{NetworkId, OutputRef};
use spectrum_offchain::domain::Has;
use spectrum_offchain::persistent_index::PersistentIndex;
use spectrum_offchain_cardano::data::pool::PoolValidation;
use spectrum_offchain_cardano::deployment::DeployedScriptInfo;
use spectrum_offchain_cardano::deployment::ProtocolValidator::*;
use spectrum_offchain_cardano::event_pipeline::read_events::read_events;
use splash_dao_offchain::deployment::ProtocolValidator as DaoProtocolValidator;
use splash_dao_offchain::protocol_config::{
    BufferWalletAuthPolicy, OperatorCreds, PermManagerAuthPolicy, SplashPolicy, WPFactoryAuthPolicy,
};
use splash_dao_offchain::GenesisEpochStartTime;
use splash_yf_offchain::settings::MinLovelacePerHarvest;
use std::collections::HashSet;

pub mod log_events;
pub mod resolve_gauges;

pub async fn event_pipeline<U, Log, Cx, Utxos, Gauges>(
    upstream: U,
    log: Log,
    context: Cx,
    utxos: Utxos,
    gauges: Gauges,
    utxo_filter: HashSet<ScriptHash>,
) where
    U: Stream<
        Item = (
            BlockEvents<Either<BabbageTransaction, Transaction>>,
            TransactionHandle,
        ),
    >,
    Log: EventLog + Accounts,
    Utxos: PersistentIndex<OutputRef, TimedOutput>,
    Gauges: VoteEscrowIndex,
    Cx: Has<DeployedScriptInfo<{ ConstFnPoolV1 as u8 }>>
        + Has<DeployedScriptInfo<{ ConstFnPoolV2 as u8 }>>
        + Has<DeployedScriptInfo<{ ConstFnPoolFeeSwitch as u8 }>>
        + Has<DeployedScriptInfo<{ ConstFnPoolFeeSwitchV2 as u8 }>>
        + Has<DeployedScriptInfo<{ ConstFnPoolFeeSwitchBiDirFee as u8 }>>
        + Has<DeployedScriptInfo<{ BalanceFnPoolV1 as u8 }>>
        + Has<DeployedScriptInfo<{ BalanceFnPoolV2 as u8 }>>
        + Has<DeployedScriptInfo<{ StableFnPoolT2T as u8 }>>
        + Has<DeployedScriptInfo<{ RoyaltyPoolV1 as u8 }>>
        + Has<PoolValidation>
        + Has<DeployedScriptInfo<{ DaoProtocolValidator::WpFactory as u8 }>>
        + Has<DeployedScriptInfo<{ DaoProtocolValidator::SmartFarm as u8 }>>
        + Has<DeployedScriptInfo<{ DaoProtocolValidator::PermManager as u8 }>>
        + Has<DeployedScriptInfo<{ DaoProtocolValidator::HarvestOrder as u8 }>>
        + Has<DeployedScriptInfo<{ DaoProtocolValidator::MintWpAuthPolicy as u8 }>>
        + Has<DeployedScriptInfo<{ DaoProtocolValidator::BufferWallet as u8 }>>
        + Has<GenesisEpochStartTime>
        + Has<PoolValidation>
        + Has<BufferWalletAuthPolicy>
        + Has<PermManagerAuthPolicy>
        + Has<WPFactoryAuthPolicy>
        + Has<SplashPolicy>
        + Has<OperatorCreds>
        + Has<NetworkId>
        + Has<MinLovelacePerHarvest>
        + 'static,
{
    log_onchain_events(
        upstream.then(|(block, tx_handle)| {
            read_events(block, &context, &utxos, &utxo_filter)
                .then(|batch| translate_events(batch, &gauges))
                .map(|events| (events, tx_handle))
        }),
        &log,
    )
    .await
}

pub async fn process_mature_events<DB: MatureEvents>(db: DB) {
    loop {
        if !db.try_process_mature_events().await {
            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
        }
    }
}
