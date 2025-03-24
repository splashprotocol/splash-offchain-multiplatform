use crate::onchain::event::{
    FarmActivated, FarmCreated, FarmDeactivated, FarmEvent, OnChainEvent, PollFactoryUpdated,
    StatelessOnChainEvent,
};
use crate::ve_index::VoteEscrowIndex;
use cardano_chain_sync::atomic_flow::BlockEvents;
use splash_dao_offchain::entities::onchain::smart_farm::FarmId;
use std::collections::HashSet;

pub async fn resolve_gauges<I: VoteEscrowIndex>(
    events: BlockEvents<StatelessOnChainEvent>,
    index: &I,
) -> BlockEvents<OnChainEvent> {
    match events {
        BlockEvents::RollForward { events, block_num } => BlockEvents::RollForward {
            events: resolve_events(events, index).await,
            block_num,
        },
        BlockEvents::RollBackward { events, block_num } => BlockEvents::RollBackward {
            events: resolve_events(events, index).await,
            block_num,
        },
    }
}

async fn resolve_events<I: VoteEscrowIndex>(
    events: Vec<StatelessOnChainEvent>,
    index: &I,
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
            StatelessOnChainEvent::Account(e) => translated_events.push(OnChainEvent::Account(e)),
        }
    }
    translated_events
}
