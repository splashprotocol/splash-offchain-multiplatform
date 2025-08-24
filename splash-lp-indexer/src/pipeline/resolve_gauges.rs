use crate::onchain::event::{GaugeEvent, GaugeWeighted, OnChainEvent, StatelessOnChainEvent};
use crate::ve_index::VoteEscrowIndex;
use cardano_chain_sync::atomic_flow::BlockEvents;

pub async fn resolve_gauges<I: VoteEscrowIndex>(
    events: BlockEvents<StatelessOnChainEvent>,
    index: &I,
) -> BlockEvents<OnChainEvent> {
    match events {
        BlockEvents::RollForward {
            events,
            block_num,
            block_slot,
        } => BlockEvents::RollForward {
            events: resolve_events(events, index).await,
            block_num,
            block_slot,
        },
        BlockEvents::RollBackward {
            events,
            block_num,
            block_slot,
        } => BlockEvents::RollBackward {
            events: resolve_events(events, index).await,
            block_num,
            block_slot,
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
            StatelessOnChainEvent::Gauge(gauge_created) => {
                index
                    .bind_gauge(gauge_created.farm_id, gauge_created.pool_id)
                    .await;
                translated_events.push(OnChainEvent::Gauge(GaugeEvent::GaugeCreated(gauge_created)))
            }
            StatelessOnChainEvent::WeightingPoll(weighting_poll_completed) => {
                for (gauge, weight) in weighting_poll_completed.distribution {
                    if let Some(pool_id) = index.get_gauge_binding(gauge).await {
                        translated_events.push(OnChainEvent::Gauge(GaugeEvent::GaugeWeighted(
                            GaugeWeighted {
                                pool_id,
                                weight,
                                epoch: weighting_poll_completed.epoch,
                            },
                        )))
                    }
                }
            }
            StatelessOnChainEvent::Position(e) => translated_events.push(OnChainEvent::Account(e)),
            StatelessOnChainEvent::Pool(e) => translated_events.push(OnChainEvent::Pool(e)),
        }
    }
    translated_events
}
