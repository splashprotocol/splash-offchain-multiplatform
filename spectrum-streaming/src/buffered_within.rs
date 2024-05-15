use futures::Stream;
use futures_core::ready;
use futures_timer::Delay;
use pin_project_lite::pin_project;
use std::future::Future;
use std::mem;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Duration;

pin_project! {
    #[derive(Debug)]
    #[must_use = "streams do nothing unless polled"]
    pub struct BufferedWithin<S: Stream> {
        #[pin]
        stream: S,
        #[pin]
        timer: Delay,
        duration: Duration,
    }
}

impl<S: Stream> BufferedWithin<S> {
    pub fn new(stream: S, duration: Duration) -> Self {
        Self {
            stream,
            timer: Delay::new(duration),
            duration,
        }
    }
}

impl<S: Stream> Stream for BufferedWithin<S> {
    type Item = S::Item;
    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let mut this = self.as_mut().project();
        let d = this.timer.as_mut();
        if d.poll(cx).is_ready() {
            // Start polling upstream only once the buffering time has elapsed.
            match ready!(this.stream.as_mut().poll_next(cx)) {
                // If no accumulated items left in the upstream reset the timer.
                None => {
                    let _ = mem::replace(&mut *this.timer, Delay::new(*this.duration));
                }
                // Otherwise keep returning accumulated items.
                ready_item => return Poll::Ready(ready_item),
            }
        }
        Poll::Pending
    }
}
