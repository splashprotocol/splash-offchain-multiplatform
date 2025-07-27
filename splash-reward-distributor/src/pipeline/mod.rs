use cardano_chain_sync::atomic_flow::{BlockEvents, TransactionHandle};
use cml_chain::transaction::{Transaction, TransactionOutput};
use cml_crypto::ScriptHash;
use cml_multi_era::babbage::BabbageTransaction;
use either::Either;
use futures::{FutureExt, Stream, StreamExt};
use spectrum_cardano_lib::OutputRef;
use spectrum_offchain::domain::Has;
use spectrum_offchain::persistent_index::PersistentIndex;
use spectrum_offchain_cardano::deployment::DeployedScriptInfo;
use spectrum_offchain_cardano::event_pipeline::read_events::read_events;
use splash_dao_offchain::deployment::ProtocolValidator as DaoProtocolValidator;
use splash_dao_offchain::entities::onchain::smart_farm::FarmId;
use splash_dao_offchain::protocol_config::{
    BufferWalletScript, FarmAuthPolicy, PermManagerAuthPolicy, SplashPolicy, WPFactoryAuthPolicy,
};
use std::collections::HashSet;

use crate::config::HarvestLimits;
use crate::events::OnChainEvents;

pub async fn event_pipeline<U, Cx, Utxos>(
    upstream: U,
    context: Cx,
    utxos: Utxos,
    utxo_filter: HashSet<ScriptHash>,
) where
    U: Stream<
        Item = (
            BlockEvents<Either<BabbageTransaction, Transaction>>,
            TransactionHandle,
        ),
    >,
    Utxos: PersistentIndex<OutputRef, TransactionOutput>,
    Cx: Has<DeployedScriptInfo<{ DaoProtocolValidator::SmartFarm as u8 }>>
        + Has<DeployedScriptInfo<{ DaoProtocolValidator::HarvestOrder as u8 }>>
        + Has<BufferWalletScript>
        + Has<DeployedScriptInfo<{ DaoProtocolValidator::PermManager as u8 }>>
        + Has<HarvestLimits>
        + Has<SplashPolicy>
        + Has<PermManagerAuthPolicy>
        + Has<FarmAuthPolicy>,
{
    upstream
        .then(|(block, tx_handle)| {
            read_events::<OnChainEvents<FarmId, OutputRef, TransactionOutput>, _, _>(
                block,
                &context,
                &utxos,
                &utxo_filter,
            )
            .map(|batch| (batch, tx_handle))
        })
        .for_each(|(_, tx_handle)| async move { tx_handle.commit() })
        .await
}
