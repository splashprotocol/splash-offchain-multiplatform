use crate::config::HarvestLimits;
use crate::entity_index::rocksdb::OnChainIndex;
use crate::entity_index::{index_events, AuthManagerIndex, BufferWalletIndex, GaugeIndex, HarvestOrderIndex};
use crate::events::OnChainEvent;
use cardano_chain_sync::atomic_flow::{BlockEvents, TransactionHandle};
use cml_chain::transaction::{Transaction, TransactionOutput};
use cml_crypto::ScriptHash;
use cml_multi_era::babbage::BabbageTransaction;
use either::Either;
use futures::{FutureExt, Sink, SinkExt, Stream, StreamExt, TryFutureExt};
use spectrum_cardano_lib::output::FinalizedTxOut;
use spectrum_cardano_lib::tx_view::TimedOutput;
use spectrum_cardano_lib::{NetworkId, OutputRef};
use spectrum_offchain::domain::Has;
use spectrum_offchain::persistent_index::PersistentIndex;
use spectrum_offchain_cardano::deployment::DeployedScriptInfo;
use spectrum_offchain_cardano::event_pipeline::read_events::read_events;
use spectrum_streaming::run_stream;
use splash_dao_offchain::deployment::ProtocolValidator as DaoProtocolValidator;
use splash_dao_offchain::entities::onchain::smart_farm::FarmId;
use splash_dao_offchain::funding::FundingRepo;
use splash_dao_offchain::protocol_config::{
    BufferWalletScript, FarmAuthPolicy, OperatorCreds, PermManagerAuthPolicy, SplashPolicy,
    WPFactoryAuthPolicy,
};
use std::collections::HashSet;

pub async fn event_pipeline<U, S, Cx, Utxos, I, F>(
    upstream: U,
    sink: S,
    context: Cx,
    indexer: I,
    funding: F,
    utxos: Utxos,
    utxo_filter: HashSet<ScriptHash>,
) where
    U: Stream<
        Item = (
            BlockEvents<Either<BabbageTransaction, Transaction>>,
            TransactionHandle,
        ),
    >,
    S: Sink<(
            BlockEvents<OnChainEvent<FarmId, OutputRef, FinalizedTxOut>>,
            TransactionHandle,
        )> + Unpin
        + Clone,
    Utxos: PersistentIndex<OutputRef, TimedOutput>,
    Cx: Has<DeployedScriptInfo<{ DaoProtocolValidator::SmartFarm as u8 }>>
        + Has<DeployedScriptInfo<{ DaoProtocolValidator::HarvestOrder as u8 }>>
        + Has<BufferWalletScript>
        + Has<DeployedScriptInfo<{ DaoProtocolValidator::PermManager as u8 }>>
        + Has<HarvestLimits>
        + Has<NetworkId>
        + Has<OperatorCreds>
        + Has<SplashPolicy>
        + Has<PermManagerAuthPolicy>
        + Has<FarmAuthPolicy>,
    I: HarvestOrderIndex<OutputRef, FinalizedTxOut>
        + BufferWalletIndex<OutputRef, FinalizedTxOut>
        + GaugeIndex<FarmId, OutputRef, FinalizedTxOut>
        + AuthManagerIndex<FarmId, OutputRef, FinalizedTxOut>
        + Clone,
    F: FundingRepo + Clone,
{
    let _ = upstream
        .then(|(block, tx_handle)| {
            read_events(block, &context, &utxos, &utxo_filter)
                .then(|batch| index_events(batch, &indexer, &funding))
                .map(|batch| Ok((batch, tx_handle)))
        })
        .forward(sink)
        .await;
}
