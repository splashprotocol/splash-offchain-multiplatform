use crate::events::OnChainEvent;
use futures::Stream;

pub async fn handle_events<U, Q, StateId, Box>(upstream: U, tasks: Q)
where
    U: Stream<Item = OnChainEvent<StateId, Box>>,
{
}
