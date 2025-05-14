use crate::onchain::event::OnChainEvent::MultipleHarvest;
use bloom_offchain::execution_engine::bundled::Bundled;
use cml_chain::address::Address;
use cml_chain::assets::AssetName;
use cml_chain::certs::{Credential, StakeCredential};
use cml_chain::transaction::TransactionOutput;
use cml_crypto::{Ed25519KeyHash, ScriptHash};
use derive_more::{Display, From, Into};
use serde::{Deserialize, Serialize};
use spectrum_cardano_lib::output::FinalizedTxOut;
use spectrum_cardano_lib::transaction::TransactionOutputExtension;
use spectrum_cardano_lib::value::ValueExtension;
use spectrum_offchain::domain::Has;
use spectrum_offchain::ledger::TryFromLedger;
use spectrum_offchain_cardano::data::order::Order;
use spectrum_offchain_cardano::deployment::DeployedScriptInfo;
use splash_dao_offchain::constants::SPLASH_NAME;
use splash_dao_offchain::deployment::ProtocolValidator;
use splash_dao_offchain::entities::onchain::smart_farm::SmartFarmSnapshot;
use splash_dao_offchain::entities::Snapshot;
use splash_dao_offchain::protocol_config::{FarmAuthPolicy, PermManagerAuthPolicy, SplashPolicy};
use splash_dao_offchain::routines::{Slot, TimedOutputRef};
use splash_distribution::entities::perm_manager_entity::PermManagerEntity;
use splash_distribution::entities::smart_farm::SmartFarm;
use splash_lp_indexer::config::HarvestLimits;
use splash_lp_indexer::onchain::event::{
    FarmCreated, MultipleAccountsHarvest, PollFactoryEvents, PoolCreated, PositionEvent, WithOptionalSlot,
};
use splash_lp_indexer::tx_view::TxViewPartiallyResolved;

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq, Debug)]
pub enum OnChainEvent {
    MultipleHarvest(MultipleAccountsHarvest),
    FarmStateUpdate(SmartFarm),
    BufferedWallet(BufferedWallet),
    PermManager(PermManagerEntity),
}

impl WithOptionalSlot for OnChainEvent {
    fn slot(&self) -> Option<Slot> {
        None
    }
}

impl<Ctx> TryFromLedger<TransactionOutput, Ctx> for OnChainEvent
where
    Ctx: Has<PermManagerAuthPolicy>
        + Has<FarmAuthPolicy>
        + Has<HarvestLimits>
        + Has<Slot>
        + Has<Vec<Ed25519KeyHash>>
        + Has<DeployedScriptInfo<{ ProtocolValidator::SmartFarm as u8 }>>
        + Has<DeployedScriptInfo<{ ProtocolValidator::HarvestOrder as u8 }>>
        + Has<DeployedScriptInfo<{ ProtocolValidator::PermManager as u8 }>>
        + Has<BufferWalletAddress>
        + Has<SplashPolicy>,
{
    fn try_from_ledger(repr: &TransactionOutput, ctx: &Ctx) -> Option<Self> {
        SmartFarm::try_from_ledger(repr, ctx)
            .map(OnChainEvent::FarmStateUpdate)
            .or_else(|| BufferedWallet::try_from_ledger(repr, ctx).map(OnChainEvent::BufferedWallet))
            .or_else(|| {
                MultipleAccountsHarvest::try_from_ledger(repr, ctx).map(OnChainEvent::MultipleHarvest)
            })
            .or_else(|| PermManagerEntity::try_from_ledger(repr, ctx).map(OnChainEvent::PermManager))
    }
}

pub type BufferedWalletSnapshot = Snapshot<BufferedWallet, FinalizedTxOut>;

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq, Debug, Display)]
#[display(
    "BufferedWallet ( splash_amount = {}, lovelace_amount = {} )",
    splash_amount,
    lovelace_amount
)]
pub struct BufferedWallet {
    pub splash_amount: u64,
    pub lovelace_amount: u64,
}

#[derive(Clone)]
pub struct BufferWalletAddress(pub ScriptHash);

impl<C> TryFromLedger<TransactionOutput, C> for BufferedWallet
where
    C: Has<BufferWalletAddress> + Has<SplashPolicy>,
{
    fn try_from_ledger(repr: &TransactionOutput, ctx: &C) -> Option<Self> {
        if test_address(repr.address(), ctx) {
            let splash_asset_name = AssetName::try_from(SPLASH_NAME).unwrap();
            let splash_qty = repr
                .value()
                .multiasset
                .get(&ctx.select::<SplashPolicy>().0, &splash_asset_name)?;
            let ada_qty = repr.value().coin;
            return Some(BufferedWallet {
                splash_amount: splash_qty,
                lovelace_amount: ada_qty,
            });
        };
        None
    }
}

pub fn test_address<Ctx>(addr: &Address, ctx: &Ctx) -> bool
where
    Ctx: Has<BufferWalletAddress>,
{
    let maybe_hash = addr.payment_cred().and_then(|c| match c {
        StakeCredential::PubKey { .. } => None,
        StakeCredential::Script { hash, .. } => Some(hash),
    });
    //info!("Going to test maybe_hash {} against {}", maybe_hash.map(|h| h.to_hex()).unwrap_or("unknown".to_string()), ctx.get().script_hash.to_hex());
    if let Some(this_hash) = maybe_hash {
        return *this_hash == ctx.get().0;
    }
    false
}