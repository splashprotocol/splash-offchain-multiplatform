mod message;

use std::net::SocketAddr;
use std::sync::Arc;

use futures::{channel::mpsc, SinkExt, StreamExt};

use crate::message::ExecutionReport;
use async_std::{
    io::BufReader,
    net::{TcpListener, TcpStream, ToSocketAddrs},
    prelude::*,
};
use clap::Parser;
use log::{error, info};
use spectrum_offchain::reporting::REPORT_DELIMITER;
use tokio::task;
use tokio_postgres::NoTls;
use tracing_subscriber::fmt::Subscriber;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;
type Sender<T> = mpsc::UnboundedSender<T>;
type Receiver<T> = mpsc::UnboundedReceiver<T>;

#[tokio::main]
async fn main() {
    let subscriber = Subscriber::new();
    tracing::subscriber::set_global_default(subscriber).expect("setting tracing default failed");
    let args = AppArgs::parse();
    let (archiver_sender, archiver_receiver) = mpsc::unbounded();
    let archiver = spawn_and_log_error(archiver_loop(args.clone().into(), archiver_receiver));
    let _ = accept_loop(archiver_sender.clone(), args.bind_addr).await;
    drop(archiver_sender);
    let _ = archiver.await;
}

async fn accept_loop(
    archiver: Sender<(SocketAddr, ExecutionReport)>,
    addr: impl ToSocketAddrs,
) -> Result<()> {
    let listener = TcpListener::bind(addr).await?;
    let mut incoming = listener.incoming();
    while let Some(stream) = incoming.next().await {
        let stream = stream?;
        let peer = stream.peer_addr()?;
        info!("Accepting connection from: {}", peer);
        spawn_and_log_error(connection_loop(archiver.clone(), peer, stream));
    }
    Ok(())
}

async fn connection_loop(
    mut archiver: Sender<(SocketAddr, ExecutionReport)>,
    peer: SocketAddr,
    stream: TcpStream,
) -> Result<()> {
    let stream = Arc::new(stream);
    let reader = BufReader::new(&*stream);
    let mut reports = reader
        .split(REPORT_DELIMITER)
        .map(|result| serde_json::from_slice::<ExecutionReport>(result.unwrap().as_slice()));
    while let Some(report) = reports.next().await {
        archiver.send((peer, report?)).await?;
    }

    Ok(())
}

const INSERT_ST: &str = "INSERT INTO executions (id, ver, pair, price_num, price_den, removed_input, added_output, side, meta, reporter) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)";

async fn archiver_loop(pg: Pg, mut reports: Receiver<(SocketAddr, ExecutionReport)>) -> Result<()> {
    let url = format!(
        "host={} port={} user={} password={}",
        pg.host, pg.port, pg.user, pg.pass
    );
    println!("Connecting");
    let (client, conn) = tokio_postgres::connect(url.as_str(), NoTls).await?;
    println!("Connected");
    task::spawn(conn);
    loop {
        let (reporter, report) = reports.select_next_some().await;
        for exec in report.executions {
            let (price_num, price_den) = exec.mean_price.unwrap().reduced().into_raw();
            let meta = serde_json::to_string(&report.meta)?;
            client
                .execute(
                    INSERT_ST,
                    &[
                        &exec.id.to_string(),
                        &exec.version.to_string(),
                        &report.pair.to_string(),
                        &(price_num as i64),
                        &(price_den as i64),
                        &(exec.removed_input as i64),
                        &(exec.added_output as i64),
                        &exec.side.to_string(),
                        &meta,
                        &reporter.to_string(),
                    ],
                )
                .await?;
        }
    }
}

fn spawn_and_log_error<F>(fut: F) -> task::JoinHandle<()>
where
    F: Future<Output = Result<()>> + Send + 'static,
{
    task::spawn(async move {
        if let Err(e) = fut.await {
            error!("{}", e)
        }
    })
}

struct Pg {
    host: String,
    port: u16,
    user: String,
    pass: String,
}

impl From<AppArgs> for Pg {
    fn from(args: AppArgs) -> Self {
        Self {
            host: args.host,
            port: args.port,
            user: args.user,
            pass: args.pass,
        }
    }
}

#[derive(Parser, Clone)]
#[command(name = "splash-engine-telemetry")]
#[command(author = "Spectrum Labs")]
#[command(version = "1.0.0")]
struct AppArgs {
    #[arg(long)]
    host: String,
    #[arg(long)]
    port: u16,
    #[arg(long)]
    user: String,
    #[arg(long)]
    pass: String,
    #[arg(long)]
    bind_addr: String,
}
