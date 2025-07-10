mod handle_events;

use cardano_chain_sync::atomic_flow::{BlockEvents, TransactionHandle};
use cml_chain::transaction::{Transaction, TransactionOutput};
use cml_crypto::ScriptHash;
use cml_multi_era::babbage::BabbageTransaction;
use either::Either;
use futures::{Stream, StreamExt};
use spectrum_cardano_lib::OutputRef;
use spectrum_offchain::domain::Has;
use spectrum_offchain::persistent_index::PersistentIndex;
use spectrum_offchain_cardano::deployment::DeployedScriptInfo;
use spectrum_offchain_cardano::event_pipeline::read_events::read_events;
use splash_dao_offchain::deployment::ProtocolValidator;
use splash_dao_offchain::protocol_config::{FarmAuthPolicy, PermManagerAuthPolicy, WPFactoryAuthPolicy};
use std::collections::HashSet;

pub async fn event_pipeline<U, Cx, Utxos, Gauges>(
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
    Cx: Has<DeployedScriptInfo<{ ProtocolValidator::SmartFarm as u8 }>>
        + Has<DeployedScriptInfo<{ ProtocolValidator::HarvestOrder as u8 }>>
        + Has<PermManagerAuthPolicy>
        + Has<WPFactoryAuthPolicy>
        + Has<FarmAuthPolicy>,
{
    //upstream.then(|(block, tx_handle)| read_events(block, &context, &utxos, &utxo_filter))
}
