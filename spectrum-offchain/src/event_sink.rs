use std::sync::Arc;

use futures::stream::StreamExt;
use futures::Stream;
use tokio::sync::Mutex;

use crate::event_sink::event_handler::{DefaultEventHandler, EventHandler};

pub mod event_handler;

pub fn process_events<'a, TUpstream, TEvent, TDefHan>(
    upstream: TUpstream,
    handlers: Vec<Box<dyn EventHandler<TEvent>>>,
    default_han: TDefHan,
) -> impl Stream<Item = ()> + 'a
where
    TUpstream: Stream<Item = TEvent> + 'a,
    TEvent: Clone + 'a,
    TDefHan: DefaultEventHandler<TEvent> + 'a,
{
    let handlers_arc = Arc::new(Mutex::new(handlers));
    let def_handler_arc = Arc::new(Mutex::new(default_han));
    upstream.then(move |ev| {
        let hans = handlers_arc.clone();
        let def_han = def_handler_arc.clone();
        async move {
            let mut unhandled_ev = Some(ev.clone());
            let mut hans_guard = hans.lock().await;
            for han in hans_guard.iter_mut() {
                println!("Going to handle");
                let maybe_unhandled_ev = han.try_handle(ev.clone()).await;
                if unhandled_ev.is_some() {
                    unhandled_ev = maybe_unhandled_ev;
                }
            }
            if let Some(unhandled_ev) = unhandled_ev {
                let mut def_han_guard = def_han.lock().await;
                def_han_guard.handle(unhandled_ev).await;
            }
        }
    })
}
