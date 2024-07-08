use std::pin::Pin;
use std::task::{Context, Poll};

use futures_core::Stream;
use pin_project_lite::pin_project;

pin_project! {
    #[derive(Debug)]
    #[must_use = "streams do nothing unless polled"]
    pub struct Conditional<S: Stream, C> {
        #[pin]
        stream: S,
        cond: C,
    }
}

impl<S, C> Conditional<S, C>
where
    S: Stream,
    C: Fn() -> bool,
{
    pub fn new(stream: S, cond: C) -> Self {
        Self { stream, cond }
    }
}

impl<S, C> Stream for Conditional<S, C>
where
    S: Stream,
    C: Fn() -> bool,
{
    type Item = S::Item;
    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        if !(&self.cond)() {
            return Poll::Pending;
        }
        self.as_mut().project().stream.as_mut().poll_next(cx)
    }
}
