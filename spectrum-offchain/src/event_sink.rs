use std::sync::Arc;

use futures::stream::StreamExt;
use futures::Stream;
use tokio::sync::Mutex;

use crate::event_sink::event_handler::EventHandler;

pub mod event_handler;

pub fn process_events<'a, TUpstream, TEvent>(
    upstream: TUpstream,
    handlers: Vec<Box<dyn EventHandler<TEvent> + Send>>,
) -> impl Stream<Item = ()> + Send + 'a
where
    TUpstream: Stream<Item = TEvent> + Send + 'a,
    TEvent: Send + 'a,
{
    let handlers_arc = Arc::new(Mutex::new(handlers));
    upstream.then(move |ev| {
        let hans = handlers_arc.clone();
        async move {
            let mut unhandled_ev = Some(ev);
            let mut hans_guard = hans.lock().await;
            // Apply handlers one by one until the event is handled.
            for han in hans_guard.iter_mut() {
                if let Some(ev) = unhandled_ev.take() {
                    unhandled_ev = han.try_handle(ev).await;
                } else {
                    break;
                }
            }
        }
    })
}
