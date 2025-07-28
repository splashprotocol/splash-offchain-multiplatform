use crate::onchain::buffer_wallet::BufferWallet;
use crate::onchain::harvest_order::{get_consumed_harvest_orders, try_new_harvest_request, HarvestOrder};
use crate::onchain::smart_farm::Gauge;
use crate::{config::HarvestLimits, onchain::auth_manager::AuthManager};
use cml_chain::transaction::TransactionOutput;
use cml_core::Slot;
use cml_crypto::Ed25519KeyHash;
use spectrum_cardano_lib::tx_view::TxViewPartiallyResolved;
use spectrum_cardano_lib::OutputRef;
use spectrum_offchain::domain::Has;
use spectrum_offchain::ledger::TryFromLedger;
use spectrum_offchain_cardano::deployment::DeployedScriptInfo;
use splash_dao_offchain::protocol_config::{BufferWalletScript, SplashPolicy};
use splash_dao_offchain::routines::Slot;
use splash_dao_offchain::{
    deployment::ProtocolValidator as DaoProtocolValidator,
    entities::onchain::smart_farm::FarmId,
    protocol_config::{FarmAuthPolicy, PermManagerAuthPolicy},
    routines::TimedOutputRef,
};

#[derive(Debug, Clone, PartialEq)]
pub struct SettledEvent<GaugeId, StateId, Bearer>(OnChainEvent<GaugeId, StateId, Bearer>, Slot);

#[derive(Debug, Clone, PartialEq)]
pub enum OnChainEvent<GaugeId, StateId, Bearer> {
    NewHarvestRequest(HarvestOrder<StateId>),
    HarvestRequestCancelled(StateId),
    Harvested(HarvestOrder<StateId>),
    BufferWalletUpdated(EntityUpdated<BufferWallet<StateId>, StateId, Bearer>),
    GaugeUpdated(EntityUpdated<Gauge<GaugeId, StateId>, StateId, Bearer>),
    AuthManagerUpdated(EntityUpdated<AuthManager<GaugeId, StateId>, StateId, Bearer>),
}

pub struct OnChainEvents<GaugeId, StateId, Bearer>(pub Vec<OnChainEvent<GaugeId, StateId, Bearer>>);

impl<Cx> TryFromLedger<TxViewPartiallyResolved, Cx> for OnChainEvents<FarmId, OutputRef, TransactionOutput>
where
    Cx: Has<PermManagerAuthPolicy>
        + Has<FarmAuthPolicy>
        + Has<HarvestLimits>
        + Has<SplashPolicy>
        + Has<PermManagerAuthPolicy>
        + Has<DeployedScriptInfo<{ DaoProtocolValidator::SmartFarm as u8 }>>
        + Has<BufferWalletScript>
        + Has<DeployedScriptInfo<{ DaoProtocolValidator::HarvestOrder as u8 }>>
        + Has<DeployedScriptInfo<{ DaoProtocolValidator::PermManager as u8 }>>,
{
    fn try_from_ledger(repr: &TxViewPartiallyResolved, ctx: &Cx) -> Option<Self> {
        type BufferWalletUpdate = EntityUpdated<BufferWallet<OutputRef>, OutputRef, TransactionOutput>;
        type GaugeUpdate = EntityUpdated<Gauge<FarmId, OutputRef>, OutputRef, TransactionOutput>;
        type AuthManagerUpdate = EntityUpdated<AuthManager<FarmId, OutputRef>, OutputRef, TransactionOutput>;

        let mut events = vec![];
        let mut buffer_wallet_found = false;

        if let Some(buffer_wallet_update) = BufferWalletUpdate::try_from_ledger(repr, ctx) {
            events.push(OnChainEvent::BufferWalletUpdated(buffer_wallet_update));
            buffer_wallet_found = true;
        }

        let consumed_harvest_orders = get_consumed_harvest_orders(repr, ctx);

        // Make sure to process inputs first for harvest orders
        if buffer_wallet_found {
            // Batch harvest TX
            for output_ref in consumed_harvest_orders {
                events.push(OnChainEvent::Harvested(output_ref));
            }
        } else {
            // Harvest order is refunded in this TX iff BufferWallet isn't present.
            for order in consumed_harvest_orders {
                events.push(OnChainEvent::HarvestRequestCancelled(order.id));
            }
        }

        if let Some(new_harvest_order) = try_new_harvest_request(repr, ctx) {
            events.push(OnChainEvent::NewHarvestRequest(new_harvest_order));
        }

        if let Some(gauge) = GaugeUpdate::try_from_ledger(repr, ctx) {
            events.push(OnChainEvent::GaugeUpdated(gauge));
        }

        if let Some(auth) = AuthManagerUpdate::try_from_ledger(repr, ctx) {
            events.push(OnChainEvent::AuthManagerUpdated(auth));
        }

        Some(OnChainEvents(events))
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct EntityUpdated<Entity, StateId, Bearer> {
    pub consumed: Option<StateId>,
    pub created: (Entity, Bearer),
}
