use async_std::task::ready;
use futures::Sink;
use std::collections::VecDeque;
use std::fmt::Debug;
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

impl<T: ?Sized, Item> BatchSinkExt<Item> for T where T: Sink<Item> {}

pub trait BatchSinkExt<Item>: Sink<Item> {
    fn batch_send(&mut self, items: Vec<Item>) -> BatchSend<'_, Self, Item>
    where
        Self: Unpin,
    {
        assert_future(BatchSend {
            sink: self,
            items: items.into(),
        })
    }
}

impl<T: ?Sized, Key, Item> KeyedBatchSinkExt<Key, Item> for T where T: Sink<(Key, Item)> {}

pub trait KeyedBatchSinkExt<Key, Item>: Sink<(Key, Item)> {
    fn batch_send_by_key(&mut self, key: Key, items: Vec<Item>) -> BatchSendByKey<'_, Self, Key, Item>
    where
        Self: Unpin,
        Key: Copy,
    {
        assert_future(BatchSendByKey {
            sink: self,
            key,
            items: items.into(),
        })
    }
}

#[derive(Debug)]
pub struct BatchSend<'a, Sink: ?Sized, Item> {
    sink: &'a mut Sink,
    items: VecDeque<Item>,
}

// Pinning is never projected to children
impl<Si: Unpin + ?Sized, Item> Unpin for BatchSend<'_, Si, Item> {}

impl<Si: Sink<Item> + Unpin + ?Sized, Item> Future for BatchSend<'_, Si, Item> {
    type Output = Result<(), Si::Error>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        let mut sink = Pin::new(&mut this.sink);
        loop {
            if this.items.is_empty() {
                break;
            }
            ready!(sink.as_mut().poll_ready(cx))?;
            let item = this.items.pop_front().unwrap();
            sink.as_mut().start_send(item)?;
        }
        ready!(sink.as_mut().poll_flush(cx))?;
        Poll::Ready(Ok(()))
    }
}

#[derive(Debug)]
pub struct BatchSendByKey<'a, Sink: ?Sized, Key, Item> {
    sink: &'a mut Sink,
    key: Key,
    items: VecDeque<Item>,
}

// Pinning is never projected to children
impl<Si: Unpin + ?Sized, Key, Item> Unpin for BatchSendByKey<'_, Si, Key, Item> {}

impl<Si: Sink<(Key, Item)> + Unpin + ?Sized, Key: Copy, Item> Future for BatchSendByKey<'_, Si, Key, Item> {
    type Output = Result<(), Si::Error>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        let mut sink = Pin::new(&mut this.sink);
        loop {
            if this.items.is_empty() {
                break;
            }
            ready!(sink.as_mut().poll_ready(cx))?;
            let item = this.items.pop_front().unwrap();
            sink.as_mut().start_send((this.key, item))?;
        }
        ready!(sink.as_mut().poll_flush(cx))?;
        Poll::Ready(Ok(()))
    }
}

// Just a helper function to ensure the futures we're returning all have the
// right implementations.
fn assert_future<T, F>(future: F) -> F
where
    F: Future<Output = T>,
{
    future
}
