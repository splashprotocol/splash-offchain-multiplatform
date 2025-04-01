use crate::onchain::event::{
    AccountEvent, FarmActivated, FarmCreated, FarmDeactivated, FarmEvent, Harvest, OnChainEvent,
    PollFactoryUpdated, StatelessOnChainEvent,
};
use crate::position_db::accounts::Accounts;
use crate::ve_index::VoteEscrowIndex;
use cardano_chain_sync::atomic_flow::BlockEvents;
use splash_dao_offchain::entities::onchain::smart_farm::FarmId;
use std::collections::HashSet;

pub async fn resolve_gauges<I: VoteEscrowIndex, DB: Accounts>(
    events: BlockEvents<StatelessOnChainEvent>,
    index: &I,
    accounts: &DB,
) -> BlockEvents<OnChainEvent> {
    match events {
        BlockEvents::RollForward { events, block_num } => BlockEvents::RollForward {
            events: resolve_events(events, index, accounts).await,
            block_num,
        },
        BlockEvents::RollBackward { events, block_num } => BlockEvents::RollBackward {
            events: resolve_events(events, index, accounts).await,
            block_num,
        },
    }
}

async fn resolve_events<I: VoteEscrowIndex, DB: Accounts>(
    events: Vec<StatelessOnChainEvent>,
    index: &I,
    accounts: &DB,
) -> Vec<OnChainEvent> {
    let mut translated_events = vec![];
    for ev in events {
        match ev {
            StatelessOnChainEvent::FarmCreated(FarmCreated { farm_id, pool_id }) => {
                index.put_gauge(farm_id, pool_id).await;
            }
            StatelessOnChainEvent::PollFactoryUpdated(PollFactoryUpdated { new_state }) => {
                if let Some(old_state) = index.get_poll_factory_snapshot().await {
                    let old_gauges: HashSet<FarmId> = HashSet::from_iter(old_state.active_farms);
                    let new_gauges = HashSet::from_iter(new_state.active_farms.clone());
                    let removed_gauges = old_gauges.difference(&new_gauges);
                    for gauge in removed_gauges {
                        if let Some(pool_id) = index.get_gauge_binding(*gauge).await {
                            translated_events.push(OnChainEvent::FarmEvent(FarmEvent::FarmDeactivated(
                                FarmDeactivated { pool_id },
                            )))
                        }
                    }
                    let added_gauges = new_gauges.difference(&old_gauges);
                    for gauge in added_gauges {
                        if let Some(pool_id) = index.get_gauge_binding(*gauge).await {
                            translated_events.push(OnChainEvent::FarmEvent(FarmEvent::FarmActivated(
                                FarmActivated { pool_id },
                            )))
                        }
                    }
                }
                index.update_poll_factory_snapshot(new_state).await;
            }
            StatelessOnChainEvent::Position(e) => {
                translated_events.push(OnChainEvent::Account(AccountEvent::Position(e)))
            }
            StatelessOnChainEvent::MultipleHarvest(multiple_harvest) => {
                for account in multiple_harvest.accounts {
                    let account_pools = accounts.get_account_pools(account.clone()).await;
                    for pool_id in account_pools {
                        translated_events.push(OnChainEvent::Account(AccountEvent::Harvest(Harvest {
                            pool_id,
                            account: account.clone(),
                            harvested_till: multiple_harvest.harvested_till.0,
                        })))
                    }
                }
            }
        }
    }
    translated_events
}
