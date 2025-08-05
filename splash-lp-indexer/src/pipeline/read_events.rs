use bloom_offchain::execution_engine::bundled::Bundled;
use cardano_chain_sync::atomic_flow::BlockEvents;
use cml_chain::address::Address;
use cml_chain::certs::StakeCredential;
use cml_chain::transaction::{Transaction, TransactionOutput};
use cml_crypto::ScriptHash;
use cml_multi_era::babbage::BabbageTransaction;
use either::Either;
use futures::{stream, StreamExt};
use spectrum_cardano_lib::tx_view::{TxView, TxViewPartiallyResolved};
use spectrum_cardano_lib::OutputRef;
use spectrum_offchain::domain::Has;
use spectrum_offchain::ledger::TryFromLedger;
use spectrum_offchain::persistent_index::PersistentIndex;
use spectrum_offchain_cardano::data::pool::PoolValidation;
use spectrum_offchain_cardano::deployment::DeployedScriptInfo;
use spectrum_offchain_cardano::deployment::ProtocolValidator::{
    BalanceFnPoolV1, BalanceFnPoolV2, ConstFnPoolFeeSwitch, ConstFnPoolFeeSwitchBiDirFee,
    ConstFnPoolFeeSwitchV2, ConstFnPoolV1, ConstFnPoolV2, RoyaltyPoolV1, StableFnPoolT2T,
};
use splash_dao_offchain::deployment::ProtocolValidator as DaoProtocolValidator;
use splash_dao_offchain::protocol_config::{
    BufferWalletScript, FarmAuthPolicy, PermManagerAuthPolicy, SplashPolicy, WPFactoryAuthPolicy,
};
use splash_dao_offchain::routines::Slot;
use splash_reward_distributor::config::HarvestLimits;
use splash_reward_distributor::indexer::{HarvestOrderIndex, Mod};
use std::collections::{HashMap, HashSet};
use type_equalities::IsEqual;

pub async fn read_events<'a, Out, Cx, Index, Harvest>(
    mut block: BlockEvents<Either<BabbageTransaction, Transaction>>,
    context: &'a Cx,
    index: &Index,
    harvest_order_index: &Harvest,
    utxo_filter: &HashSet<ScriptHash>,
) -> BlockEvents<Out>
where
    Cx: 'static,
    Out: TryFromLedger<TxViewPartiallyResolved, WithHarvestOrderCreationSlots<'a, Cx>>,
    Index: PersistentIndex<OutputRef, TransactionOutput>,
    Harvest: HarvestOrderIndex<OutputRef, TransactionOutput>,
{
    let (txs, slot) = match &mut block {
        BlockEvents::RollForward {
            events, block_slot, ..
        }
        | BlockEvents::RollBackward {
            events, block_slot, ..
        } => (events.drain(0..), block_slot),
    };

    let txs = stream::iter(txs)
        .map(TxView::from)
        .then(|tx| async move {
            index_utxos(&tx, index, utxo_filter).await;
            tx
        })
        .then(|tx| TxViewPartiallyResolved::resolve(tx, index, *slot))
        .collect::<Vec<_>>()
        .await
        .into_iter();

    let mut events = vec![];

    for tx in txs {
        let mut map_to_slots = HashMap::new();
        for (input, _) in &tx.inputs {
            let id = OutputRef::from(input.clone());
            if let Some(wrapped_order) = harvest_order_index.read_harvest_order(id).await {
                let slot = match wrapped_order {
                    Mod::Confirmed(Bundled(t, _)) | Mod::Predicted(Bundled(t, _)) => t.created_at_slot,
                };
                map_to_slots.insert(id, slot);
            }
        }

        let map_to_slots = HarvestOrderCreationSlots(map_to_slots);
        let augmented_ctx = WithHarvestOrderCreationSlots(context, map_to_slots);
        if let Some(out) = Out::try_from_ledger(&tx, &augmented_ctx) {
            events.push(out);
        }
    }

    block.map(|_| events)
}

async fn index_utxos<Index: PersistentIndex<OutputRef, TransactionOutput>>(
    tx: &TxView,
    index: &Index,
    utxo_filter: &HashSet<ScriptHash>,
) {
    for (ix, o) in tx.outputs.iter().enumerate() {
        if test_address(o.address(), utxo_filter) {
            let oref = OutputRef::new(tx.hash, ix as u64);
            index.insert(oref, o.clone()).await;
        }
    }
}

pub fn test_address(addr: &Address, utxo_filter: &HashSet<ScriptHash>) -> bool {
    let maybe_hash = addr.payment_cred().and_then(|c| match c {
        StakeCredential::PubKey { .. } => None,
        StakeCredential::Script { hash, .. } => Some(hash),
    });
    if let Some(this_hash) = maybe_hash {
        return utxo_filter.contains(&this_hash);
    }
    false
}

#[derive(Clone)]
/// Given a collection of TX inputs `I`, this struct represents a mapping of harvest order
/// `OutputRef` values in `I` to the Slot time of the TX which created the harvest order.
pub struct HarvestOrderCreationSlots(pub HashMap<OutputRef, Slot>);

pub struct WithHarvestOrderCreationSlots<'a, Cx>(pub &'a Cx, pub HarvestOrderCreationSlots);

impl<Cx> Has<HarvestOrderCreationSlots> for WithHarvestOrderCreationSlots<'_, Cx> {
    fn select<U: IsEqual<HarvestOrderCreationSlots>>(&self) -> HarvestOrderCreationSlots {
        self.1.clone()
    }
}

macro_rules! impl_has {
    ($t:ty) => {
        impl<'a, Cx> Has<$t> for WithHarvestOrderCreationSlots<'a, Cx>
        where
            Cx: Has<$t>,
        {
            fn select<U: IsEqual<$t>>(&self) -> $t {
                self.0.select::<U>()
            }
        }
    };
}

impl_has!(BufferWalletScript);
impl_has!(DeployedScriptInfo<{ ConstFnPoolV1 as u8 }>);
impl_has!(DeployedScriptInfo<{ ConstFnPoolV2 as u8 }>);
impl_has!(DeployedScriptInfo<{ ConstFnPoolFeeSwitch as u8 }>);
impl_has!(DeployedScriptInfo<{ ConstFnPoolFeeSwitchV2 as u8 }>);
impl_has!(DeployedScriptInfo<{ ConstFnPoolFeeSwitchBiDirFee as u8 }>);
impl_has!(DeployedScriptInfo<{ BalanceFnPoolV1 as u8 }>);
impl_has!(DeployedScriptInfo<{ BalanceFnPoolV2 as u8 }>);
impl_has!(DeployedScriptInfo<{ StableFnPoolT2T as u8 }>);
impl_has!(DeployedScriptInfo<{ RoyaltyPoolV1 as u8 }>);
impl_has!(DeployedScriptInfo<{ DaoProtocolValidator::WpFactory as u8 }>);
impl_has!(DeployedScriptInfo<{ DaoProtocolValidator::SmartFarm as u8 }>);
impl_has!(DeployedScriptInfo<{ DaoProtocolValidator::PermManager as u8 }>);
impl_has!(DeployedScriptInfo<{ DaoProtocolValidator::HarvestOrder as u8 }>);
impl_has!(PoolValidation);
impl_has!(PermManagerAuthPolicy);
impl_has!(WPFactoryAuthPolicy);
impl_has!(FarmAuthPolicy);
impl_has!(SplashPolicy);
impl_has!(HarvestLimits);
