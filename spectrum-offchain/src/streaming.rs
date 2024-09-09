use std::future::Future;
use std::pin::Pin;

use futures::Stream;
use futures::StreamExt;

pub fn boxed<'a, T>(s: impl Stream<Item = T> + 'a) -> Pin<Box<dyn Stream<Item = T> + 'a>> {
    Box::pin(s)
}

pub fn run_stream<S: Stream>(stream: S) -> impl Future<Output = ()> {
    async move {
        stream.collect::<Vec<_>>().await;
    }
}
