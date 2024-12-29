use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering::Relaxed;
use std::sync::Arc;
use std::task::{Context, Poll};

pub fn once(flag: Arc<AtomicBool>) -> Once {
    Once(flag)
}

/// A future that completes once underlying [AtomicBool] is `true`.
pub struct Once(Arc<AtomicBool>);

impl Future for Once {
    type Output = ();
    fn poll(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Self::Output> {
        if self.0.load(Relaxed) {
            return Poll::Ready(());
        }
        Poll::Pending
    }
}
