mod confirm_txs;

use crate::accounts::Accounts;
use crate::entity_index::{index_events, AuthManagerIndex, BufferWalletIndex, GaugeIndex, HarvestOrderIndex};
use crate::pipeline::confirm_txs::forward_confirmed_txs;
use cardano_chain_sync::atomic_flow::{BlockEvents, TransactionHandle};
use cml_chain::transaction::Transaction;
use cml_crypto::{ScriptHash, TransactionHash};
use cml_multi_era::babbage::BabbageTransaction;
use either::Either;
use futures::stream::FusedStream;
use futures::{Sink, SinkExt, Stream, StreamExt};
use spectrum_cardano_lib::output::FinalizedTxOut;
use spectrum_cardano_lib::tx_view::TimedOutput;
use spectrum_cardano_lib::{NetworkId, OutputRef};
use spectrum_offchain::domain::Has;
use spectrum_offchain::persistent_index::PersistentIndex;
use spectrum_offchain_cardano::deployment::DeployedScriptInfo;
use spectrum_offchain_cardano::event_pipeline::read_events::read_events;
use splash_dao_offchain::deployment::ProtocolValidator as DaoProtocolValidator;
use splash_dao_offchain::entities::onchain::smart_farm::FarmId;
use splash_dao_offchain::funding::FundingRepo;
use splash_dao_offchain::protocol_config::{
    BufferWalletAuthPolicy, OperatorCreds, PermManagerAuthPolicy, SplashPolicy,
};
use splash_dao_offchain::GenesisEpochStartTime;
use splash_yf_offchain::events::OnChainEvent;
use splash_yf_offchain::settings::MinLovelacePerHarvest;
use std::collections::HashSet;
use std::fmt::Debug;

pub async fn event_pipeline<U, S, Tx, Cx, Utxos, I, F>(
    mut upstream: U,
    mut sink: S,
    confirmed_txs: Tx,
    context: Cx,
    indexer: I,
    funding: F,
    utxos: Utxos,
    utxo_filter: HashSet<ScriptHash>,
) where
    U: Stream<
            Item = (
                BlockEvents<Either<BabbageTransaction, Transaction>>,
                Option<TransactionHandle>,
            ),
        > + FusedStream
        + Unpin,
    S: Sink<(
            BlockEvents<OnChainEvent<FarmId, OutputRef, FinalizedTxOut>>,
            Option<TransactionHandle>,
        )> + Unpin
        + Clone,
    Tx: Sink<(TransactionHash, u64)> + Unpin + Clone,
    Tx::Error: Debug,
    Utxos: PersistentIndex<OutputRef, TimedOutput>,
    Cx: Has<DeployedScriptInfo<{ DaoProtocolValidator::SmartFarm as u8 }>>
        + Has<DeployedScriptInfo<{ DaoProtocolValidator::HarvestOrder as u8 }>>
        + Has<DeployedScriptInfo<{ DaoProtocolValidator::BufferWallet as u8 }>>
        + Has<DeployedScriptInfo<{ DaoProtocolValidator::PermManager as u8 }>>
        + Has<BufferWalletAuthPolicy>
        + Has<MinLovelacePerHarvest>
        + Has<NetworkId>
        + Has<OperatorCreds>
        + Has<SplashPolicy>
        + Has<GenesisEpochStartTime>
        + Has<PermManagerAuthPolicy>,
    I: HarvestOrderIndex<OutputRef, FinalizedTxOut>
        + BufferWalletIndex<OutputRef, FinalizedTxOut>
        + GaugeIndex<FarmId, OutputRef, FinalizedTxOut>
        + AuthManagerIndex<FarmId, OutputRef, FinalizedTxOut>
        + Clone,
    F: FundingRepo + Clone,
{
    loop {
        let (block, tx_handle) = upstream.select_next_some().await;
        forward_confirmed_txs(&block, confirmed_txs.clone()).await;
        let batch = read_events(block, &context, &utxos, &utxo_filter).await;

        let genesis_start_time = context.select::<GenesisEpochStartTime>();
        let network_id = context.select::<NetworkId>();
        let batch = index_events(batch, &indexer, &funding, genesis_start_time, network_id).await;
        let _ = sink.send((batch, tx_handle)).await;
    }
}
