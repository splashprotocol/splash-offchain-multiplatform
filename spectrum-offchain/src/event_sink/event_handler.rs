use async_trait::async_trait;

#[async_trait(?Send)]
pub trait EventHandler<TEvent> {
    /// Tries to handle the given event if applicable.
    /// Returns `Some(TEvent)` back otherwise.
    async fn try_handle(&mut self, ev: TEvent) -> Option<TEvent>;
}

#[async_trait(?Send)]
pub trait DefaultEventHandler<TEvent> {
    async fn handle<'a>(&mut self, ev: TEvent)
    where
        TEvent: 'a;
}

pub struct NoopDefaultHandler;

#[async_trait(?Send)]
impl<TEvent> DefaultEventHandler<TEvent> for NoopDefaultHandler {
    async fn handle<'a>(&mut self, _ev: TEvent)
    where
        TEvent: 'a,
    {
    }
}
