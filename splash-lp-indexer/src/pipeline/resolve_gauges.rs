use crate::onchain::event::{
    ActivePools, GaugeWeighted, OnChainEvent, StatelessOnChainEvent, SuspendedPools,
};
use crate::onchain::GaugeWeight;
use crate::ve_index::VoteEscrowIndex;
use cardano_chain_sync::atomic_flow::BlockEvents;

pub async fn translate_events<I: VoteEscrowIndex>(
    events: BlockEvents<StatelessOnChainEvent>,
    index: &I,
) -> BlockEvents<OnChainEvent> {
    match events {
        BlockEvents::RollForward {
            events,
            block_num,
            block_slot,
        } => BlockEvents::RollForward {
            events: resolve_gauges(events, index).await,
            block_num,
            block_slot,
        },
        BlockEvents::RollBackward {
            events,
            block_num,
            block_slot,
        } => BlockEvents::RollBackward {
            events: resolve_gauges(events, index).await,
            block_num,
            block_slot,
        },
    }
}

async fn resolve_gauges<I: VoteEscrowIndex>(
    events: Vec<StatelessOnChainEvent>,
    index: &I,
) -> Vec<OnChainEvent> {
    let mut translated_events = vec![];
    for ev in events {
        match ev {
            StatelessOnChainEvent::Gauge(gauge_created) => {
                index
                    .bind_gauge(gauge_created.farm_id, gauge_created.pool_id)
                    .await;
            }
            StatelessOnChainEvent::WeightingPoll(weighting_poll_completed) => {
                if let Some(total_poll_weight) = weighting_poll_completed.total_poll_weight {
                    for (gauge, weight) in weighting_poll_completed.distribution {
                        if let Some(pool_id) = index.get_gauge_binding(gauge).await {
                            translated_events.push(OnChainEvent::Gauge(GaugeWeighted {
                                pool_id,
                                weight: GaugeWeight(weight, total_poll_weight),
                                epoch: weighting_poll_completed.epoch,
                            }))
                        }
                    }
                } else {
                    // Newly-created weighting poll with no votes cast yet. Extract active pools for the epoch.
                    let mut active_pools = vec![];
                    for (gauge, _) in weighting_poll_completed.distribution {
                        if let Some(pool_id) = index.get_gauge_binding(gauge).await {
                            if !active_pools.contains(&pool_id) {
                                active_pools.push(pool_id);
                            }
                        }
                    }
                    translated_events.push(OnChainEvent::NewWeightingPoll(ActivePools(
                        weighting_poll_completed.epoch,
                        active_pools,
                    )));
                }
            }
            StatelessOnChainEvent::Position(e) => translated_events.push(OnChainEvent::Account(e)),
            StatelessOnChainEvent::Pool(e) => translated_events.push(OnChainEvent::Pool(e)),
            StatelessOnChainEvent::PermManager(perm_manager_update) => {
                let mut suspended_pools = vec![];
                for farm_id in perm_manager_update.suspended_farms {
                    if let Some(pool_id) = index.get_gauge_binding(farm_id).await {
                        if !suspended_pools.contains(&pool_id) {
                            suspended_pools.push(pool_id);
                        }
                    }
                }
                translated_events.push(OnChainEvent::PermManagerUpdate(SuspendedPools(suspended_pools)))
            }
        }
    }
    translated_events
}
