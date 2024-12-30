pub mod buffered_within;
pub mod conditional;

use std::time::Duration;

use futures_core::Stream;

use crate::buffered_within::BufferedWithin;
use crate::conditional::Conditional;

use async_std::task::ready;
use pin_project_lite::pin_project;
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

pub fn boxed<'a, T>(s: impl Stream<Item = T> + 'a) -> Pin<Box<dyn Stream<Item = T> + 'a>> {
    Box::pin(s)
}

pub fn run_stream<S: Stream>(stream: S) -> impl Future<Output = ()> {
    RunStream { stream }
}

pin_project! {
    #[must_use = "futures do nothing unless you `.await` or poll them"]
    pub struct RunStream<S: Stream> {
        #[pin]
        stream: S,
    }
}

impl<S: Stream> Future for RunStream<S> {
    type Output = ();
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut this = self.project();
        loop {
            let _ = ready!(this.stream.as_mut().poll_next(cx));
        }
    }
}

impl<T: ?Sized> StreamExt for T where T: Stream {}

pub trait StreamExt: Stream {
    /// Accumulates items for at least `duration` before emitting them.
    fn buffered_within(self, duration: Duration) -> BufferedWithin<Self>
    where
        Self: Sized,
    {
        BufferedWithin::new(self, duration)
    }

    /// Check condition `cond` before pulling from upstream.
    fn conditional<F>(self, cond: F) -> Conditional<Self, F>
    where
        Self: Sized,
        F: Fn() -> bool,
    {
        Conditional::new(self, cond)
    }
}
