use crate::onchain::buffer_wallet::BufferWallet;
use crate::onchain::harvest_order::{get_consumed_harvest_orders, try_new_harvest_request, HarvestOrder};
use crate::onchain::smart_farm::{Gauge, UpdatedGauges};
use crate::{config::HarvestLimits, onchain::auth_manager::AuthManager};
use cml_chain::transaction::TransactionOutput;
use cml_crypto::Ed25519KeyHash;
use spectrum_cardano_lib::transaction::TransactionOutputExtension;
use spectrum_cardano_lib::tx_view::TxViewPartiallyResolved;
use spectrum_cardano_lib::value::ValueExtension;
use spectrum_cardano_lib::{AssetClass, AssetName, NetworkId, OutputRef, Token};
use spectrum_offchain::domain::Has;
use spectrum_offchain::ledger::TryFromLedger;
use spectrum_offchain_cardano::deployment::DeployedScriptInfo;
use splash_dao_offchain::constants::SPLASH_NAME;
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OnChainEvent<GaugeId, StateId, Bearer> {
    BotHarvestingAction {
        payouts: Vec<(HarvestOrder<StateId>, SplashPayout)>,
        buffer_wallet_update: EntityUpdated<BufferWallet<StateId>, StateId, Bearer>,
    },
    BotGaugeBufferingAction {
        drained_gauges: Vec<EntityUpdated<Gauge<GaugeId, StateId>, StateId, Bearer>>,
        buffer_wallet_update: EntityUpdated<BufferWallet<StateId>, StateId, Bearer>,
    },
    UpdatedGauges(UpdatedGauges<GaugeId, StateId, Bearer>),
    AuthManagerUpdated(EntityUpdated<AuthManager<GaugeId, StateId>, StateId, Bearer>),
    NewHarvestRequest(HarvestOrder<StateId>),
    HarvestRequestCancelled(Vec<StateId>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SplashPayout(pub u64);

impl<Cx> TryFromLedger<TxViewPartiallyResolved, Cx> for OnChainEvent<FarmId, OutputRef, TransactionOutput>
where
    Cx: Has<PermManagerAuthPolicy>
        + Has<FarmAuthPolicy>
        + Has<HarvestLimits>
        + Has<NetworkId>
        + Has<SplashPolicy>
        + Has<PermManagerAuthPolicy>
        + Has<DeployedScriptInfo<{ DaoProtocolValidator::SmartFarm as u8 }>>
        + Has<BufferWalletScript>
        + Has<DeployedScriptInfo<{ DaoProtocolValidator::HarvestOrder as u8 }>>
        + Has<DeployedScriptInfo<{ DaoProtocolValidator::PermManager as u8 }>>,
{
    fn try_from_ledger(repr: &TxViewPartiallyResolved, ctx: &Cx) -> Option<Self> {
        type BufferWalletUpdate = EntityUpdated<BufferWallet<OutputRef>, OutputRef, TransactionOutput>;
        type AuthManagerUpdate = EntityUpdated<AuthManager<FarmId, OutputRef>, OutputRef, TransactionOutput>;
        let network_id = ctx.select::<NetworkId>();

        let splash_asset_name = AssetName::from_utf8(SPLASH_NAME.into());
        let splash_policy = ctx.select::<SplashPolicy>().0;
        let splash_asset_class = AssetClass::Token(Token(splash_policy, splash_asset_name));

        let consumed_harvest_orders = get_consumed_harvest_orders(repr, ctx);

        // Make sure to process inputs first for harvest orders
        if let Some(buffer_wallet_update) = BufferWalletUpdate::try_from_ledger(repr, ctx) {
            if !consumed_harvest_orders.is_empty() {
                // Batch harvesting tx
                let mut payouts = vec![];

                for harvest_order in consumed_harvest_orders {
                    let address = harvest_order.address(network_id);
                    if let Some(payout_output) = &repr
                        .outputs
                        .iter()
                        .find(|tx_output| *tx_output.address() == address)
                    {
                        let splash_payout =
                            SplashPayout(payout_output.value().amount_of(splash_asset_class)?);
                        payouts.push((harvest_order, splash_payout));
                    }
                }
                Some(OnChainEvent::BotHarvestingAction {
                    payouts,
                    buffer_wallet_update,
                })
            } else {
                // gauge-buffering tx
                let Some(gauge_updates) = UpdatedGauges::try_from_ledger(repr, ctx) else {
                    unreachable!("Can't update buffer wallet with no harvest orders nor any gauge updates");
                };
                Some(OnChainEvent::BotGaugeBufferingAction {
                    drained_gauges: gauge_updates.0,
                    buffer_wallet_update,
                })
            }
        } else if !consumed_harvest_orders.is_empty() {
            // Harvest order is refunded in this TX iff BufferWallet isn't present. Note that there
            // exists an edge case where some of these harvest orders might not even be known to the
            // bot. This is because if the bot witnesses multiple-created harvest orders in a single
            // TX, only the first one is acknowledged.
            let res = consumed_harvest_orders
                .into_iter()
                .map(|order| order.id)
                .collect();
            Some(OnChainEvent::HarvestRequestCancelled(res))
        } else if let Some(new_harvest_order) = try_new_harvest_request(repr, ctx) {
            Some(OnChainEvent::NewHarvestRequest(new_harvest_order))
        } else if let Some(updated_gauges) = UpdatedGauges::try_from_ledger(repr, ctx) {
            Some(OnChainEvent::UpdatedGauges(updated_gauges))
        } else {
            AuthManagerUpdate::try_from_ledger(repr, ctx).map(OnChainEvent::AuthManagerUpdated)
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntityUpdated<Entity, StateId, Bearer> {
    pub consumed: Option<StateId>,
    pub created: (Entity, Bearer),
}
