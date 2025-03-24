use crate::position_db::export_feed::ExportEventFeed;
use log::trace;
use rdkafka::producer::{FutureProducer, FutureRecord};
use serde::Serialize;
use std::marker::PhantomData;
use std::time::Duration;

pub struct EventPublisher<E, Q> {
    queue: Q,
    kafka: FutureProducer,
    topic: String,
    pd: PhantomData<E>,
}

impl<E, Q> EventPublisher<E, Q> {
    pub fn new(queue: Q, kafka: FutureProducer, topic: String) -> Self {
        Self {
            queue,
            kafka,
            topic,
            pd: PhantomData,
        }
    }
}

const POLL_INTERVAL: Duration = Duration::from_secs(3);

impl<E, Q> EventPublisher<E, Q> {
    pub async fn run(self)
    where
        E: Serialize + Send,
        Q: ExportEventFeed,
    {
        loop {
            while let Some((key, event)) = self.queue.next().await {
                let event_bytes = serde_json::to_vec(&event).unwrap();
                trace!("Exporting event: {}", serde_json::to_string(&event).unwrap());
                let record = FutureRecord::<(), _>::to(self.topic.as_str()).payload(&event_bytes);
                self.kafka.send(record, Duration::from_secs(0)).await.unwrap();
                self.queue.delete(key).await;
            }
            tokio::time::sleep(POLL_INTERVAL).await;
        }
    }
}
