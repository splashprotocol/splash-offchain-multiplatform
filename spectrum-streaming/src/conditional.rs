use std::mem;
use std::pin::Pin;
use std::task::{Context, Poll};

use futures_core::Stream;
use log::trace;
use pin_project_lite::pin_project;

pin_project! {
    #[derive(Debug)]
    #[must_use = "streams do nothing unless polled"]
    pub struct Conditional<S: Stream, C> {
        #[pin]
        stream: S,
        cond: C,
        is_suspended: bool,
    }
}

impl<S, C> Conditional<S, C>
where
    S: Stream,
    C: Fn() -> bool,
{
    pub fn new(stream: S, cond: C) -> Self {
        Self {
            stream,
            cond,
            is_suspended: false,
        }
    }
}

impl<S, C> Stream for Conditional<S, C>
where
    S: Stream,
    C: Fn() -> bool,
{
    type Item = S::Item;
    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let mut this = self.as_mut().project();
        if !(this.cond)() {
            let _ = mem::replace(this.is_suspended, true);
            trace!("Conditional stream suspended");
            return Poll::Pending;
        }
        if mem::replace(this.is_suspended, false) {
            trace!("Conditional stream resumed");
        }
        this.stream.as_mut().poll_next(cx)
    }
}
