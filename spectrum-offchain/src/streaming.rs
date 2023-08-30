use std::pin::Pin;

use futures::Stream;

pub fn boxed<'a, T>(s: impl Stream<Item = T> + 'a) -> Pin<Box<dyn Stream<Item = T> + 'a>> {
    Box::pin(s)
}
