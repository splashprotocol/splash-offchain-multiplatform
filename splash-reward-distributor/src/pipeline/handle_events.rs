use crate::events::OnChainEvent;
use futures::Stream;

pub async fn handle_events<U, Q, Box>(upstream: U, tasks: Q)
where
    U: Stream<Item = OnChainEvent<Box>>,
{
}
