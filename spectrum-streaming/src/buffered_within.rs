use futures::Stream;
use futures_timer::Delay;
use pin_project_lite::pin_project;
use std::collections::VecDeque;
use std::future::Future;
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
        // Shortcut for zero buffering
        if this.duration.is_zero() {
            this.stream.as_mut().poll_next(cx)
        } else {
            if let Poll::Ready(Some(item)) = this.stream.as_mut().poll_next(cx) {
                this.buffer.push_back(item);
            }
            if this.timer.as_mut().poll(cx).is_ready() {
                if let Some(buffered_item) = this.buffer.pop_front() {
                    // Keep returning accumulated items util the buffer is exhausted.
                    return Poll::Ready(Some(buffered_item));
                } else {
                    // If no accumulated items left in the upstream reset the timer.
                    this.timer.reset(*this.duration);
                }
            }
            Poll::Pending
        }
    }
}

#[cfg(test)]
mod tests {
    use super::BufferedWithin;
    use futures::channel::mpsc;
    use futures::{SinkExt, StreamExt};
    use tokio::time::{timeout, Duration};

    #[tokio::test]
    async fn test_buffering_with_mpsc_and_timings() {
        let (mut tx, rx) = mpsc::unbounded();
        let mut buffered_stream = BufferedWithin::new(rx, Duration::from_millis(200));

        // Spawn a task to simulate sending events.
        tokio::spawn(async move {
            tx.send(1).await.unwrap();
            tokio::time::sleep(Duration::from_millis(100)).await;

            tx.send(2).await.unwrap();
            tokio::time::sleep(Duration::from_millis(100)).await;

            tx.send(3).await.unwrap();
            tokio::time::sleep(Duration::from_millis(300)).await;

            tx.send(4).await.unwrap();
            tx.send(5).await.unwrap();
        });

        let mut results = Vec::new();

        while let Ok(Some(item)) = timeout(Duration::from_secs(2), buffered_stream.next()).await {
            results.push(item);
        }

        // Ensure the test did not timeout and all events were collected.
        assert_eq!(results, vec![1, 2, 3, 4, 5]);
    }
}
