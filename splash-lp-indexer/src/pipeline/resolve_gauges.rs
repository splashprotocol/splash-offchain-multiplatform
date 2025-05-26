use crate::onchain::event::{
    AccountEvent, FarmActivated, FarmCreated, FarmDeactivated, FarmEvent, Harvest, OnChainEvent,
    PollFactoryEvents, PoolEvent, StatelessOnChainEvent,
};
use crate::position_db::accounts::Accounts;
use crate::position_db::pool_frames::PoolFrames;
use crate::ve_index::VoteEscrowIndex;
use cardano_chain_sync::atomic_flow::BlockEvents;
use log::info;
use splash_dao_offchain::entities::onchain::smart_farm::FarmId;
use splash_dao_offchain::routines::Slot;
use std::collections::HashSet;

pub async fn resolve_gauges<I: VoteEscrowIndex, DB: Accounts + PoolFrames>(
    events: BlockEvents<StatelessOnChainEvent>,
    index: &I,
    events_log: &DB,
) -> BlockEvents<OnChainEvent> {
    match events {
        BlockEvents::RollForward {
            events,
            block_num,
            block_slot,
        } => BlockEvents::RollForward {
            events: resolve_events(events, index, events_log, block_slot).await,
            block_num,
            block_slot,
        },
        BlockEvents::RollBackward {
            events,
            block_num,
            block_slot,
        } => BlockEvents::RollBackward {
            events: resolve_events(events, index, events_log, block_slot).await,
            block_num,
            block_slot,
        },
    }
}

async fn resolve_events<I: VoteEscrowIndex, DB: Accounts + PoolFrames>(
    events: Vec<StatelessOnChainEvent>,
    index: &I,
    events_log: &DB,
    block_slot: u64,
) -> Vec<OnChainEvent> {
    let mut translated_events = vec![];
    for ev in events {
        match ev {
            StatelessOnChainEvent::FarmCreated(FarmCreated { farm_id, pool_id }) => {
                info!(
                    "Processing farm created event for farm {} and pool id {} at slot {}",
                    farm_id, pool_id, block_slot
                );
                if events_log.get_pool_lq_supply(pool_id).await.is_some() {
                    index.put_gauge(farm_id, pool_id).await;
                    if let Some(gauge_pre_activation_slot) = index.get_pre_activated_gauge(farm_id).await {
                        info!(
                            "Gauge {} was pre activated at {} and pool id {}",
                            farm_id, gauge_pre_activation_slot, pool_id
                        );
                        index.delete_pre_activated_gauge(farm_id).await;
                        translated_events.push(OnChainEvent::FarmEvent(FarmEvent::FarmActivated(
                            FarmActivated {
                                pool_id,
                                slot: Slot(gauge_pre_activation_slot),
                            },
                        )))
                    }
                } else {
                    info!(
                        "Attempt to create farm for non-existent pool {}, farm_id {}",
                        pool_id, farm_id
                    );
                }
            }
            StatelessOnChainEvent::PollFactory(factory_event) => {
                let mut previous_active_farms = None;
                let mut new_state;
                match factory_event {
                    PollFactoryEvents::NewFactory(poll_state) => {
                        previous_active_farms = Some(vec![]);
                        new_state = poll_state
                    }
                    PollFactoryEvents::FactoryStateUpdate(updated_factory_state) => {
                        previous_active_farms = index
                            .get_poll_factory_snapshot()
                            .await
                            .map(|old_state| old_state.active_farms);
                        new_state = updated_factory_state.new_state
                    }
                }

                if let Some(previous_active_farms) = previous_active_farms {
                    let old_gauges: HashSet<FarmId> = HashSet::from_iter(previous_active_farms);
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
                                FarmActivated {
                                    pool_id,
                                    slot: Slot(block_slot),
                                },
                            )))
                        } else {
                            index.add_pre_activated_gauge(gauge.clone(), block_slot).await;
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
                    let account_pools = events_log.get_account_pools(account.clone()).await;
                    for pool_id in account_pools {
                        translated_events.push(OnChainEvent::Account(AccountEvent::Harvest(Harvest {
                            pool_id,
                            account: account.clone(),
                            harvested_till: multiple_harvest.harvested_till.0,
                        })))
                    }
                }
            }
            StatelessOnChainEvent::PoolCreated(e) => {
                translated_events.push(OnChainEvent::PoolEvent(PoolEvent::PoolCreated(e)))
            }
        }
    }
    translated_events
}
