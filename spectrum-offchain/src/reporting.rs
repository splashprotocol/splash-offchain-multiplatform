use async_std::net::TcpStream;
use async_std::prelude::Stream;
use async_stream::stream;
use futures::channel::mpsc;
use futures::{AsyncWriteExt, SinkExt, StreamExt};
use log::warn;
use serde::Serialize;
use std::net::SocketAddr;

#[async_trait::async_trait]
pub trait Reporting<Rep> {
    async fn process_report(&mut self, report: Rep);
}

#[derive(Clone)]
pub struct ReportingChannel<Rep>(mpsc::Sender<Rep>);

#[async_trait::async_trait]
impl<Rep: Send> Reporting<Rep> for ReportingChannel<Rep> {
    async fn process_report(&mut self, report: Rep) {
        self.0.send(report).await.unwrap();
    }
}

pub struct ReportingAgent<Rep> {
    endpoint: Option<TcpStream>,
    mailbox: mpsc::Receiver<Rep>,
}

impl<Rep> ReportingAgent<Rep> {
    pub async fn new(endpoint: SocketAddr, buffer_size: usize) -> (Self, ReportingChannel<Rep>) {
        let (snd, recv) = mpsc::channel(buffer_size);
        let agent = ReportingChannel(snd);
        let endpoint = TcpStream::connect(endpoint).await.ok();
        if endpoint.is_none() {
            warn!("Reporting endpoint is inaccessible!");
        }
        (
            Self {
                endpoint,
                mailbox: recv,
            },
            agent,
        )
    }
}

pub const REPORT_DELIMITER: u8 = b'@';

pub fn reporting_stream<'a, Rep: Serialize + 'a>(
    mut agent: ReportingAgent<Rep>,
) -> impl Stream<Item = ()> + 'a {
    stream! {
        loop {
            let report = agent.mailbox.select_next_some().await;
            if let Some(ref mut endpoint) = agent.endpoint {
                let mut msg = serde_json::to_vec(&report).unwrap();
                msg.push(REPORT_DELIMITER);
                if let Err(err) = endpoint.write_all(msg.as_ref()).await {
                    warn!("Failed to submit report: {}", err);
                }
            }
        }
    }
}
