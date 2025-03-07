use rdkafka::producer::{FutureProducer, FutureRecord};
use serde::Serialize;
use std::marker::PhantomData;
use std::time::Duration;
use crate::db::account_feed::AccountEventFeed;

pub struct EventPublisher<'a, E, Q> {
    queue: Q,
    pd: PhantomData<E>,
    kafka: FutureProducer,
    topic: &'a str,
}

const POLL_INTERVAL: Duration = Duration::from_secs(3);

impl<'a, E, Q> EventPublisher<'a, E, Q> {
    async fn run(self)
    where
        E: Serialize + Send,
        Q: AccountEventFeed,
    {
        loop {
            while let Some((key, event)) = self.queue.next().await {
                let event_bytes = serde_json::to_vec(&event).unwrap();
                let record = FutureRecord::<(), _>::to(self.topic).payload(&event_bytes);
                self.kafka.send(record, Duration::from_secs(0)).await.unwrap();
                self.queue.delete(key).await;
            }
            tokio::time::sleep(POLL_INTERVAL).await;
        }
    }
}
