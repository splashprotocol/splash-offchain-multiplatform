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
/// Consumes event, so must be put last in handlers array.
pub fn forward_with<S, F>(sink: S, transform: F) -> ForwardWith<S, F> {
    ForwardWith { sink, transform }
}

pub struct ForwardWith<S, F> {
    sink: S,
    transform: F,
}

#[async_trait]
impl<A, B, S, F> EventHandler<A> for ForwardWith<S, F>
where
    A: Send + 'static,
    B: Send,
    S: Sink<B> + Unpin + Send,
    S::Error: Debug,
    F: Send + Fn(A) -> B,
{
    async fn try_handle(&mut self, ev: A) -> Option<A> {
        self.sink.send((self.transform)(ev)).await.unwrap();
        None
    }
}

/// Forward events from upstream to [S] applying transformation [F].
/// Consumes event, so must be put last in handlers array.
pub fn forward_with_ref<S, F>(sink: S, transform: F) -> ForwardWithRef<S, F> {
    ForwardWithRef { sink, transform }
}

pub struct ForwardWithRef<S, F> {
    sink: S,
    transform: F,
}

#[async_trait]
impl<A, B, S, F> EventHandler<A> for ForwardWithRef<S, F>
where
    A: Send + 'static,
    B: Send,
    S: Sink<B> + Unpin + Send,
    S::Error: Debug,
    F: Send + Fn(&A) -> B,
{
    async fn try_handle(&mut self, ev: A) -> Option<A> {
        self.sink.send((self.transform)(&ev)).await.unwrap();
        Some(ev)
    }
}

/// Forward some events from upstream to [S] applying transformation [F] that depends on state [T].
pub fn try_forward_with<S, T, F>(sink: S, state: T, transform: F) -> TryForwardWithState<S, F, T> {
    TryForwardWithState {
        sink,
        transform,
        state,
    }
}

pub struct TryForwardWithState<S, F, T> {
    sink: S,
    transform: F,
    state: T,
}

#[async_trait]
impl<A, B, S, F, T> EventHandler<A> for TryForwardWithState<S, F, T>
where
    A: Send + 'static,
    B: Send,
    S: Sink<B> + Unpin + Send,
    S::Error: Debug,
    F: Send + Fn(T, &A) -> Option<(B, T)>,
    T: Copy + Send,
{
    async fn try_handle(&mut self, ev: A) -> Option<A> {
        if let Some((result, new_state)) = (self.transform)(self.state, &ev) {
            self.state = new_state;
            self.sink.send(result).await.unwrap();
        }
        Some(ev)
    }
}
