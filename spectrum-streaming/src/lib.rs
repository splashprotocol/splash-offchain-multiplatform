use std::time::Duration;

use futures_core::Stream;

use crate::buffered_within::BufferedWithin;
use crate::conditional::Conditional;

pub mod buffered_within;
pub mod conditional;

impl<T: ?Sized> StreamExt for T where T: Stream {}

pub trait StreamExt: Stream {
    /// Accumulates items for at least `duration` before emitting them.
    fn buffered_within(self, duration: Duration) -> BufferedWithin<Self>
    where
        Self: Sized,
    {
        BufferedWithin::new(self, duration)
    }

    /// Check condition `cond` before pulling from upstream.
    fn conditional<F>(self, cond: F) -> Conditional<Self, F>
    where
        Self: Sized,
        F: Fn() -> bool,
    {
        Conditional::new(self, cond)
    }
}
