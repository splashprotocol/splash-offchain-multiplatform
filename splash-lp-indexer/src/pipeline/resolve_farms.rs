use crate::onchain::event::{OnChainEvent, RawOnChainEvent};
use cardano_chain_sync::atomic_flow::BlockEvents;

pub async fn resolve_farms(events: BlockEvents<RawOnChainEvent>) -> BlockEvents<OnChainEvent> {
    todo!()
}
