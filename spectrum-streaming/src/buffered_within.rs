use futures::Stream;
use futures_timer::Delay;
use pin_project_lite::pin_project;
use std::collections::VecDeque;
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
        buffer: VecDeque<S::Item>,
    }
}

impl<S: Stream> BufferedWithin<S> {
    pub fn new(stream: S, duration: Duration) -> Self {
        Self {
            stream,
            timer: Delay::new(duration),
            duration,
            buffer: VecDeque::new(),
        }
    }
}

impl<S: Stream> Stream for BufferedWithin<S> {
    type Item = S::Item;
    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context) -> Poll<Option<Self::Item>> {
        let mut this = self.as_mut().project();
        if let Poll::Ready(Some(item)) = this.stream.as_mut().poll_next(cx) {
            this.buffer.push_back(item);
        }
        if this.timer.as_mut().poll(cx).is_ready() {
            if let Some(buffered_item) = this.buffer.pop_front() {
                // Keep returning accumulated items util buffer is exhausted.
                return Poll::Ready(Some(buffered_item));
            } else {
                // If no accumulated items left in the upstream reset the timer.
                let _ = mem::replace(&mut *this.timer, Delay::new(*this.duration));
            }
        }

        Poll::Pending
    }
}
