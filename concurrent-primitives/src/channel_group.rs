use futures::Sink;
use std::pin::Pin;
use std::task::{Context, Poll};

/// [N] channels grouped.
/// Note, order in which messages are received by the underlying channels
/// is not guaranteed to correspond to the order of channels in group.
pub struct ChannelGroupUnordered<const N: usize, C>([C; N]);

impl<const N: usize, C> ChannelGroupUnordered<N, C> {
    pub fn new(channels: [C; N]) -> Self {
        Self(channels)
    }
}

impl<const N: usize, C, T> Sink<T> for ChannelGroupUnordered<N, C>
where
    C: Sink<T> + Unpin,
    T: Clone,
{
    type Error = C::Error;

    fn poll_ready(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        let mut acc = Poll::Ready(Ok(()));
        for c in self.0.iter_mut() {
            match Sink::poll_ready(Pin::new(c), cx) {
                Poll::Ready(Ok(_)) => {}
                not_ready_or_not_ok => {
                    acc = not_ready_or_not_ok;
                    break;
                }
            }
        }
        acc
    }

    fn start_send(mut self: Pin<&mut Self>, item: T) -> Result<(), Self::Error> {
        let mut acc = Ok(());
        for c in self.0.iter_mut() {
            match Sink::start_send(Pin::new(c), item.clone()) {
                Ok(()) => (),
                err => {
                    acc = err;
                    break;
                }
            }
        }
        acc
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        let mut acc = Poll::Ready(Ok(()));
        for c in self.0.iter_mut() {
            match Sink::poll_flush(Pin::new(c), cx) {
                Poll::Ready(Ok(_)) => {}
                not_ready_or_not_ok => {
                    acc = not_ready_or_not_ok;
                    break;
                }
            }
        }
        acc
    }

    fn poll_close(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        let mut acc = Poll::Ready(Ok(()));
        for c in self.0.iter_mut() {
            match Sink::poll_close(Pin::new(c), cx) {
                Poll::Ready(Ok(_)) => {}
                not_ready_or_not_ok => {
                    acc = not_ready_or_not_ok;
                    break;
                }
            }
        }
        acc
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::channel::mpsc;
    use futures::SinkExt;
    use futures::StreamExt;

    #[tokio::test]
    async fn test_all_receivers_receive_from_senders_unordered() {
        // Create MPSC channels (sender-receiver pairs)
        let (tx1, mut rx1) = mpsc::channel::<String>(10);
        let (tx2, mut rx2) = mpsc::channel::<String>(10);
        let (tx3, mut rx3) = mpsc::channel::<String>(10);

        // Group the senders in a ChannelGroup
        let mut channel_group = ChannelGroupUnordered::new([tx1, tx2, tx3]);

        // Messages to be sent
        let messages = [
            "Message 1".to_string(),
            "Message 2".to_string(),
            "Message 3".to_string(),
        ];

        // Send messages to all senders, order does not matter
        for message in messages.iter() {
            channel_group.send(message.clone()).await.unwrap();
        }

        // Flush the channel group to make sure messages are dispatched
        channel_group.flush().await.unwrap();

        // Collect received messages
        let received1_1 = rx1.next().await.unwrap();
        let received2_1 = rx2.next().await.unwrap();
        let received3_1 = rx3.next().await.unwrap();

        let received1_2 = rx1.next().await.unwrap();
        let received2_2 = rx2.next().await.unwrap();
        let received3_2 = rx3.next().await.unwrap();

        let received1_3 = rx1.next().await.unwrap();
        let received2_3 = rx2.next().await.unwrap();
        let received3_3 = rx3.next().await.unwrap();

        // Assert all receivers received some message (order doesn't matter)
        let received_messages_1 = vec![received1_1, received1_2, received1_3];
        let received_messages_2 = vec![received2_1, received2_2, received2_3];
        let received_messages_3 = vec![received3_1, received3_2, received3_3];

        assert!(messages
            .iter()
            .all(|message| received_messages_1.contains(message)
                && received_messages_2.contains(message)
                && received_messages_3.contains(message)));
    }
}
