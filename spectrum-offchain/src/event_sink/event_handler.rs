use async_trait::async_trait;
use futures::{Sink, SinkExt};
use std::fmt::Debug;

#[async_trait]
pub trait EventHandler<TEvent> {
    /// Tries to handle the given event if applicable.
    /// Returns `Some(TEvent)` if further processing is needed.
    async fn try_handle(&mut self, ev: TEvent) -> Option<TEvent>;
}

#[async_trait(?Send)]
pub trait DefaultEventHandler<TEvent> {
    async fn handle<'a>(&mut self, ev: TEvent)
    where
        TEvent: 'a;
}

#[derive(Copy, Clone)]
pub struct NoopDefaultHandler;

#[async_trait(?Send)]
impl<TEvent> DefaultEventHandler<TEvent> for NoopDefaultHandler {
    async fn handle<'a>(&mut self, _ev: TEvent)
    where
        TEvent: 'a,
    {
    }
}

/// Forward events from upstream to [S] applying transformation [F].
/// Consumes event, so must be put last in handler array.
pub fn forward_to<S, F>(sink: S, transform: F) -> ForwardTo<S, F> {
    ForwardTo { sink, transform }
}

pub struct ForwardTo<S, F> {
    sink: S,
    transform: F,
}

#[async_trait]
impl<A, B, S, F> EventHandler<A> for ForwardTo<S, F>
where
    A: Send + 'static,
    B: Send,
    S: Sink<B> + Unpin + Send,
    S::Error: Debug,
    F: Send + Fn(A) -> B,
{
    async fn try_handle(&mut self, ev: A) -> Option<A> {
        self.sink.send((self.transform)(ev)).await.expect("");
        None
    }
}
