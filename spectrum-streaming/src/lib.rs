use std::time::Duration;

use futures_core::Stream;

use crate::buffered_within::BufferedWithin;

pub mod buffered_within;

impl<T: ?Sized> StreamExt for T where T: Stream {}

pub trait StreamExt: Stream {
    /// Accumulates items for at least `duration` before emitting them.
    fn buffered_within(self, duration: Duration) -> BufferedWithin<Self>
    where
        Self: Sized,
    {
        BufferedWithin::new(self, duration)
    }
}
