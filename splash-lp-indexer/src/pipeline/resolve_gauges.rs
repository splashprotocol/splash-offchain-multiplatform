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
                    "[Resolving gauges] Processing farm created event for farm {} and pool id {} at slot {}",
                    farm_id, pool_id, block_slot
                );
                if events_log.get_pool_lq_supply(pool_id).await.is_some() {
                    index.put_gauge(farm_id, pool_id).await;
                    if let Some(gauge_pre_activation_slot) = index.get_pre_activated_gauge(farm_id).await {
                        info!(
                            "[Resolving gauges] Gauge {} was pre activated at {} and pool id {}",
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
                        "[Resolving gauges] Attempt to create farm for non-existent pool {}, farm_id {}",
                        pool_id, farm_id
                    );
                }
            }
            StatelessOnChainEvent::PollFactory(factory_event) => {
                info!(
                    "[Resolving gauges] Processing poll factory event {:?} at slot {}",
                    factory_event, block_slot
                );
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

                info!("[Resolving gauges] Processing poll factory update. With state: {} active farms, {:?} last epoch, {} stable id", new_state.active_farms.len(), new_state.last_poll_epoch, new_state.stable_id.to_hex());

                if let Some(previous_active_farms) = previous_active_farms {
                    let old_gauges: HashSet<FarmId> = HashSet::from_iter(previous_active_farms);
                    let new_gauges = HashSet::from_iter(new_state.active_farms.clone());
                    info!("[Resolving gauges] New gauges: {}", new_gauges.len());
                    info!("[Resolving gauges] Old gauges: {}", old_gauges.len());
                    let removed_gauges = old_gauges.difference(&new_gauges);
                    info!(
                        "[Resolving gauges] Going to process removed_gauges. Qty to process: {}",
                        removed_gauges.clone().into_iter().collect::<Vec<_>>().len()
                    );
                    for gauge in removed_gauges {
                        info!(
                            "[Resolving gauges] Going to process gauge {} in removed gauges",
                            gauge
                        );
                        if let Some(pool_id) = index.get_gauge_binding(*gauge).await {
                            info!("[Resolving gauges] Pool id for gauge {} is {}", gauge, pool_id);
                            translated_events.push(OnChainEvent::FarmEvent(FarmEvent::FarmDeactivated(
                                FarmDeactivated { pool_id },
                            )))
                        }
                        info!("[Resolving gauges] Finish processing gauge {} remove", gauge);
                    }
                    info!("[Resolving gauges] Finish removed_gauges processing");
                    let added_gauges = new_gauges.difference(&old_gauges);
                    info!(
                        "[Resolving gauges] Going to process added_gauges. Qty to process: {}",
                        added_gauges.clone().into_iter().collect::<Vec<_>>().len()
                    );
                    for gauge in added_gauges {
                        info!(
                            "[Resolving gauges] Going to process gauge {} in added gauges",
                            gauge
                        );
                        if let Some(pool_id) = index.get_gauge_binding(*gauge).await {
                            info!("[Resolving gauges] Pool id for gauge {} is {}", gauge, pool_id);
                            translated_events.push(OnChainEvent::FarmEvent(FarmEvent::FarmActivated(
                                FarmActivated {
                                    pool_id,
                                    slot: Slot(block_slot),
                                },
                            )))
                        } else {
                            info!("[Resolving gauges] Pool id is missing for gauge {}. Save it as pre_activated at slot {}", gauge, block_slot);
                            index.add_pre_activated_gauge(gauge.clone(), block_slot).await;
                            info!("[Resolving gauges] Gauge {} added as pre activated", gauge);
                        }
                        info!("[Resolving gauges] Finish processing gauge {} add", gauge);
                    }
                    info!("[Resolving gauges] Finish added_gauges processing");
                }
                info!("[Resolving gauges] Save factory with state: {} active farms, {:?} last epoch, {} stable id", new_state.active_farms.len(), new_state.last_poll_epoch, new_state.stable_id.to_hex());
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
