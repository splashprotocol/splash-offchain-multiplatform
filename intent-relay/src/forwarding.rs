use crate::queue::Dequeue;
use crate::sender::Sender;
use std::time::Duration;

const POLL_INTERVAL: Duration = Duration::from_secs(3);

/// This function continuously processes items from a queue and sends them
/// to a specified sender. It operates in a loop with the following steps:
///
/// 1. Wait for the next item in the queue.
/// 2. Once an item is retrieved, send it using the provided sender.
/// 3. After sending, delete the item from the queue to indicate it has been processed.
/// 4. If no items are available in the queue, it waits for a specified poll interval before retrying.
pub async fn forwarding<T, Q: Dequeue<T>, F: Sender<T>>(queue: Q, mut sender: F) {
    loop {
        while let Some((key, item)) = queue.next().await {
            sender.send(item).await;
            queue.delete(key).await;
        }
        tokio::time::sleep(POLL_INTERVAL).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use std::collections::VecDeque;
    use std::sync::{Arc, Mutex};

    struct MockQueue<T> {
        items: Arc<Mutex<VecDeque<(u64, T)>>>,
    }

    #[async_trait]
    impl<T: Send + Sync> Dequeue<T> for MockQueue<T> {
        async fn next(&self) -> Option<(u64, T)> {
            let mut items = self.items.lock().unwrap();
            items.pop_front()
        }

        async fn delete(&self, _key: u64) {
            // No-op for this test
        }
    }

    struct MockSender<T> {
        sent_items: Arc<Mutex<Vec<T>>>,
    }

    #[async_trait]
    impl<T: Send + Sync + Clone> Sender<T> for MockSender<T> {
        async fn send(&mut self, item: T) {
            let mut sent_items = self.sent_items.lock().unwrap();
            sent_items.push(item);
        }
    }

    #[tokio::test]
    async fn test_forwarding() {
        // Arrange
        let queue = MockQueue {
            items: Arc::new(Mutex::new(VecDeque::from(vec![
                (1, "Item1".to_string()),
                (2, "Item2".to_string()),
                (3, "Item3".to_string()),
            ]))),
        };
        let sender = MockSender {
            sent_items: Arc::new(Mutex::new(vec![])),
        };

        // Clone Arc to access sent_items outside the closure
        let sent_items_arc = sender.sent_items.clone();

        // Act
        let _ = tokio::time::timeout(Duration::from_millis(100), forwarding(queue, sender)).await;

        // Assert
        let sent_items = sent_items_arc.lock().unwrap();
        assert_eq!(
            *sent_items,
            vec!["Item1".to_string(), "Item2".to_string(), "Item3".to_string()]
        );
    }
}
