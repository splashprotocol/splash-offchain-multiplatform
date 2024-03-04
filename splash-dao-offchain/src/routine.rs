use std::time::Duration;

use async_stream::stream;
use futures::Stream;
use futures_timer::Delay;

use crate::routine::ToRoutine::RetryIn;

pub enum ToRoutine {
    RetryIn(Duration),
}

#[async_trait::async_trait]
pub trait RoutineBehaviour {
    async fn attempt(&self) -> Option<ToRoutine>;
}

pub fn retry_in(dur: Duration) -> Option<ToRoutine> {
    Some(RetryIn(dur))
}

pub struct Routine<B> {
    behaviour: B,
    waker: Option<Delay>,
}

impl<B> Routine<B> {
    fn next_attempt_in(&mut self, delay: Duration) {
        let _ = self.waker.insert(Delay::new(delay));
    }
}

pub fn routine<B: RoutineBehaviour>(mut this: Routine<B>) -> impl Stream<Item = ()> {
    stream! {
        loop {
            if let Some(delay) = this.waker.take() {
                delay.await;
            }
            if let Some(to_routine) = this.behaviour.attempt().await {
                match to_routine {
                    ToRoutine::RetryIn(delay) =>
                        this.next_attempt_in(delay)
                }
            }
        }
    }
}
