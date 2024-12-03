mod message;

use std::sync::Arc;

use futures::{channel::mpsc, FutureExt, SinkExt, StreamExt, TryStreamExt};

use crate::message::ExecutionReport;
use async_std::{
    io::BufReader,
    net::{TcpListener, TcpStream, ToSocketAddrs},
    prelude::*,
    task,
};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;
type Sender<T> = mpsc::UnboundedSender<T>;
type Receiver<T> = mpsc::UnboundedReceiver<T>;

#[derive(Debug)]
enum Void {}

pub(crate) fn main() -> Result<()> {
    task::block_on(accept_loop("127.0.0.1:8080"))
}

async fn accept_loop(addr: impl ToSocketAddrs) -> Result<()> {
    let listener = TcpListener::bind(addr).await?;
    let mut incoming = listener.incoming();
    let (archiver_sender, archiver_receiver) = mpsc::unbounded();
    let archiver = spawn_and_log_error(archiver_loop(archiver_receiver));
    while let Some(stream) = incoming.next().await {
        let stream = stream?;
        println!("Accepting from: {}", stream.peer_addr()?);
        spawn_and_log_error(connection_loop(archiver_sender.clone(), stream));
    }
    drop(archiver_sender);
    archiver.await;
    Ok(())
}

const DELIM: u8 = b'#';

async fn connection_loop(mut archiver: Sender<ExecutionReport>, stream: TcpStream) -> Result<()> {
    let stream = Arc::new(stream);
    let reader = BufReader::new(&*stream);
    let mut reports = reader
        .split(DELIM)
        .map(|result| serde_json::from_slice::<ExecutionReport>(result.unwrap().as_slice()));
    while let Some(report) = reports.next().await {
        archiver.send(report?).await?;
    }

    Ok(())
}

async fn archiver_loop(mut reports: Receiver<ExecutionReport>) -> Result<()> {
    let url = "host=localhost user=postgres";
    let (client, conn) = async_postgres::connect(url.parse()?).await?;
    task::spawn(conn);
    loop {
        let report = reports.select_next_some().await;
        client.execute(
            "INSERT INTO executions (name, data) VALUES ($1, $2)",
            &[&name, &data],
        ).await?;
    }
}

fn spawn_and_log_error<F>(fut: F) -> task::JoinHandle<()>
where
    F: Future<Output = Result<()>> + Send + 'static,
{
    task::spawn(async move {
        if let Err(e) = fut.await {
            eprintln!("{}", e)
        }
    })
}
