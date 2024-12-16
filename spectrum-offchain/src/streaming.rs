use async_std::task::ready;
use futures::Stream;
use pin_project_lite::pin_project;
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

pub fn boxed<'a, T>(s: impl Stream<Item = T> + 'a) -> Pin<Box<dyn Stream<Item = T> + 'a>> {
    Box::pin(s)
}

pub fn run_stream<S: Stream>(stream: S) -> impl Future<Output = ()> {
    RunStream { stream }
}

pin_project! {
    #[must_use = "futures do nothing unless you `.await` or poll them"]
    pub struct RunStream<S: Stream> {
        #[pin]
        stream: S,
    }
}

impl<S: Stream> Future for RunStream<S> {
    type Output = ();
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut this = self.project();
        loop {
            let _ = ready!(this.stream.as_mut().poll_next(cx));
        }
    }
}
