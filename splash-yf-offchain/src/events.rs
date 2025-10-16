use crate::entities::auth_manager::AuthManager;
use crate::entities::buffer_wallet::{BufferWallet, BufferWalletUpdate};
use crate::entities::funding_box::ConfirmedFundingBoxChanges;
use crate::entities::gauge::{GaugeDeposits, GaugeWithdrawals, UpdatedGauges};
use crate::entities::harvest_order::{get_consumed_harvest_orders, try_new_harvest_request, HarvestOrder};
use crate::entities::{
    BufferWalletSplashBalanceChange, BufferWalletSplashTokenDecrease, BufferWalletSplashTokenIncrease,
};
use crate::settings::MinLovelacePerHarvest;
use cml_crypto::TransactionHash;
use spectrum_cardano_lib::output::FinalizedTxOut;
use spectrum_cardano_lib::transaction::TransactionOutputExtension;
use spectrum_cardano_lib::tx_view::TxViewPartiallyResolved;
use spectrum_cardano_lib::value::ValueExtension;
use spectrum_cardano_lib::{AssetClass, AssetName, NetworkId, OutputRef, Token};
use spectrum_offchain::domain::Has;
use spectrum_offchain::ledger::TryFromLedger;
use spectrum_offchain_cardano::deployment::DeployedScriptInfo;
use splash_dao_offchain::constants::SPLASH_NAME;
use splash_dao_offchain::protocol_config::{BufferWalletScript, OperatorCreds, SplashPolicy};
use splash_dao_offchain::routines::Slot;
use splash_dao_offchain::GenesisEpochStartTime;
use splash_dao_offchain::{
    deployment::ProtocolValidator as DaoProtocolValidator, entities::onchain::smart_farm::FarmId,
    protocol_config::PermManagerAuthPolicy,
};

#[derive(Debug, Clone, PartialEq)]
pub struct SettledEvent<GaugeId, StateId, Bearer>(OnChainEvent<GaugeId, StateId, Bearer>, Slot);

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OnChainEvent<GaugeId, StateId, Bearer> {
    BotHarvestingAction {
        payouts: Vec<(HarvestOrder<StateId>, SplashPayout)>,
        buffer_wallet_update: EntityUpdated<BufferWallet<StateId>, StateId, Bearer>,
        buffer_wallet_withdrawn_amount: BufferWalletSplashTokenDecrease,
        tx_hash: TransactionHash,
    },
    BotGaugeBufferingAction {
        drained_gauges: GaugeWithdrawals<GaugeId, StateId, Bearer>,
        buffer_wallet_update: EntityUpdated<BufferWallet<StateId>, StateId, Bearer>,
        buffer_wallet_deposited_amount: BufferWalletSplashTokenIncrease,
        tx_hash: TransactionHash,
    },
    DepositToGauges(GaugeDeposits<GaugeId, StateId, Bearer>),
    AuthManagerUpdated(EntityUpdated<AuthManager<GaugeId, StateId>, StateId, Bearer>),
    NewHarvestRequest(HarvestOrder<StateId>, Bearer),
    HarvestRequestCancelled(Vec<StateId>),
    Funding(ConfirmedFundingBoxChanges),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SplashPayout(pub u64);

impl<Cx> TryFromLedger<TxViewPartiallyResolved, Cx> for OnChainEvent<FarmId, OutputRef, FinalizedTxOut>
where
    Cx: Has<PermManagerAuthPolicy>
        + Has<MinLovelacePerHarvest>
        + Has<GenesisEpochStartTime>
        + Has<NetworkId>
        + Has<SplashPolicy>
        + Has<OperatorCreds>
        + Has<PermManagerAuthPolicy>
        + Has<DeployedScriptInfo<{ DaoProtocolValidator::SmartFarm as u8 }>>
        + Has<BufferWalletScript>
        + Has<DeployedScriptInfo<{ DaoProtocolValidator::HarvestOrder as u8 }>>
        + Has<DeployedScriptInfo<{ DaoProtocolValidator::PermManager as u8 }>>,
{
    fn try_from_ledger(repr: &TxViewPartiallyResolved, ctx: &Cx) -> Option<Self> {
        type AuthManagerUpdate = EntityUpdated<AuthManager<FarmId, OutputRef>, OutputRef, FinalizedTxOut>;
        let network_id = ctx.select::<NetworkId>();

        let splash_asset_name = AssetName::from_utf8(SPLASH_NAME.into());
        let splash_policy = ctx.select::<SplashPolicy>().0;
        let splash_asset_class = AssetClass::Token(Token(splash_policy, splash_asset_name));

        let consumed_harvest_orders = get_consumed_harvest_orders(repr, ctx);

        // Make sure to process inputs first for harvest orders
        if let Some(buffer_wallet_update) = BufferWalletUpdate::try_from_ledger(repr, ctx) {
            let tx_hash = repr.hash;
            if !consumed_harvest_orders.is_empty() {
                // Batch harvesting tx
                let mut payouts = vec![];

                for harvest_order in consumed_harvest_orders {
                    let address = harvest_order.reward_receiver.to_address(network_id);
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
                let BufferWalletSplashBalanceChange::Decrease(withdrawn_amount) =
                    buffer_wallet_update.balance_change
                else {
                    unreachable!("Buffer wallet balance change should be decreased");
                };
                Some(OnChainEvent::BotHarvestingAction {
                    payouts,
                    buffer_wallet_update: buffer_wallet_update.update,
                    buffer_wallet_withdrawn_amount: BufferWalletSplashTokenDecrease(withdrawn_amount),
                    tx_hash,
                })
            } else {
                // gauge-buffering tx
                let Some(gauge_updates) = UpdatedGauges::try_from_ledger(repr, ctx) else {
                    unreachable!("Can't update buffer wallet with no harvest orders nor any gauge updates");
                };
                if let UpdatedGauges::Withdrawals(drained_gauges) = gauge_updates {
                    let BufferWalletSplashBalanceChange::Increase(deposited_amount) =
                        buffer_wallet_update.balance_change
                    else {
                        unreachable!("Buffer wallet balance change should be an increase");
                    };
                    Some(OnChainEvent::BotGaugeBufferingAction {
                        drained_gauges,
                        buffer_wallet_update: buffer_wallet_update.update,
                        buffer_wallet_deposited_amount: BufferWalletSplashTokenIncrease(deposited_amount),
                        tx_hash,
                    })
                } else {
                    None
                }
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
        } else if let Some((new_harvest_order, output)) = try_new_harvest_request(repr, ctx) {
            Some(OnChainEvent::NewHarvestRequest(new_harvest_order, output))
        } else if let Some(updated_gauges) = UpdatedGauges::try_from_ledger(repr, ctx) {
            let UpdatedGauges::Deposits(deposits) = updated_gauges else {
                unreachable!("Gauge updates are not deposits");
            };
            Some(OnChainEvent::DepositToGauges(deposits))
        } else if let Some(updated_auth_manager) = AuthManagerUpdate::try_from_ledger(repr, ctx) {
            Some(OnChainEvent::AuthManagerUpdated(updated_auth_manager))
        } else {
            ConfirmedFundingBoxChanges::try_from_ledger(repr, ctx).map(OnChainEvent::Funding)
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntityUpdated<Entity, StateId, Bearer> {
    pub consumed: Option<StateId>,
    pub created: (Entity, Bearer),
}
