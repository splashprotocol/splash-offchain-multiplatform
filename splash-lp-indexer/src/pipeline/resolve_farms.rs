use crate::gauge_index::GaugeIndex;
use crate::onchain::event::{
    FarmActivation, FarmDeactivation, FarmEvent, OnChainEvent, RawFarmEvent, RawOnChainEvent,
};
use cardano_chain_sync::atomic_flow::BlockEvents;

pub async fn resolve_farms<I: GaugeIndex>(
    events: BlockEvents<RawOnChainEvent>,
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

async fn resolve_events<I: GaugeIndex>(events: Vec<RawOnChainEvent>, index: &I) -> Vec<OnChainEvent> {
    let mut translated_events = vec![];
    for ev in events {
        match ev {
            RawOnChainEvent::FarmEvent(farm_event) => match farm_event {
                RawFarmEvent::FarmActivation(e) => {
                    if let Some(pool_id) = index.get_gauge_binding(e.binder).await {
                        translated_events.push(OnChainEvent::FarmEvent(FarmEvent::FarmActivation(
                            FarmActivation { binder: pool_id },
                        )))
                    }
                }
                RawFarmEvent::FarmDeactivation(e) => {
                    if let Some(pool_id) = index.get_gauge_binding(e.binder).await {
                        translated_events.push(OnChainEvent::FarmEvent(FarmEvent::FarmDeactivation(
                            FarmDeactivation { binder: pool_id },
                        )))
                    }
                }
                RawFarmEvent::FarmCreation(e) => {
                    index.put_gauge(e.farm_id, e.pool_id).await;
                }
            },
            RawOnChainEvent::Account(e) => translated_events.push(OnChainEvent::Account(e)),
        }
    }
    translated_events
}
