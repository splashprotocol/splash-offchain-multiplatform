use futures::{Stream, StreamExt};
use rdkafka::producer::{FutureProducer, FutureRecord};
use serde::Serialize;
use std::marker::PhantomData;
use std::time::Duration;

pub struct EventPublisher<'a, E, In> {
    mailbox: In,
    pd: PhantomData<E>,
    kafka: FutureProducer,
    topic: &'a str,
}

impl<'a, E, In> EventPublisher<'a, E, In> {
    async fn run(mut self)
    where
        E: Serialize + Send,
        In: Stream<Item = E> + Unpin + Send,
    {
        while let Some(event) = self.mailbox.next().await {
            let event_bytes = serde_json::to_vec(&event).unwrap();
            let record = FutureRecord::<(), _>::to(self.topic).payload(&event_bytes);
            self.kafka.send(record, Duration::from_secs(0)).await.unwrap();
        }
    }
}
