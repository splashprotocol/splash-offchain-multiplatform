mod config;
mod forwarding;
mod intent;
mod queue;
mod sender;
mod server;

use crate::server::build_api_server;
use clap::Parser;
use derive_more::From;
use futures::stream::FuturesUnordered;
use futures::FutureExt;
use spectrum_streaming::run_stream;

use crate::config::AppConfig;
use crate::forwarding::forwarding;
use crate::queue::RocksDB;
use log::error;
use std::net::{IpAddr, SocketAddr};
use std::str::FromStr;

#[tokio::main]
async fn main() {
    let args = AppArgs::parse();

    log4rs::init_file(args.log4rs_path, Default::default()).unwrap();

    let raw_config = std::fs::File::open(args.config_path).expect("Cannot load configuration file");
    let config: AppConfig = serde_json::from_reader(raw_config).expect("Invalid configuration file");

    let ip_addr = IpAddr::from_str(&*args.host).expect("Invalid host address");
    let bind_addr = SocketAddr::new(ip_addr, args.port);

    let queues = config
        .endpoints
        .iter()
        .enumerate()
        .map(|(i, _)| RocksDB::new(format!("{}/{}", config.base_data_path, i)))
        .collect::<Vec<_>>();

    let processes = FuturesUnordered::new();

    for (endpoint, queue) in config.endpoints.iter().zip(queues.iter()) {
        if let Some(sender) = sender::TcpSender::new(endpoint.to_string()).await {
            let process = tokio::spawn(forwarding(queue.clone(), sender));
            processes.push(process);
        } else {
            error!("Failed to connect to endpoint {}. Skipping...", endpoint)
        }
    }

    let server = build_api_server(queues, bind_addr)
        .await
        .expect("Error setting up api server")
        .map(|r| r.unwrap());

    let server_handle = tokio::spawn(server);
    processes.push(server_handle);

    let default_panic = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        default_panic(info);
        std::process::exit(1);
    }));

    run_stream(processes).await;
}

#[derive(Parser)]
#[command(name = "intent-relay")]
#[command(author = "Spectrum Labs")]
#[command(version = "1.0.0")]
#[command(about = "Splash Intent Relay", long_about = None)]
struct AppArgs {
    /// Path to the JSON configuration file.
    #[arg(long, short)]
    config_path: String,
    #[arg(long, short)]
    log4rs_path: String,
    #[arg(long)]
    host: String,
    #[arg(long)]
    port: u16,
}
