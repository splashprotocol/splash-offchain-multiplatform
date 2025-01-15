mod server;
mod tx_submission;

use crate::server::{build_api_server, Limits};
use crate::tx_submission::TxSubmissionAgent;
use clap::Parser;
use cml_chain::transaction::Transaction;
use constants::CONWAY_ERA_ID;
use derive_more::From;
use futures::stream::FuturesUnordered;
use futures::FutureExt;
use spectrum_cardano_lib::constants;
use spectrum_offchain_cardano::node::NodeConfig;
use spectrum_streaming::run_stream;

use std::net::{IpAddr, SocketAddr};
use std::str::FromStr;

#[derive(serde::Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
struct AppConfig {
    pub node: NodeConfig,
    pub limits: Limits,
    pub tx_submission_buffer_size: usize,
}

#[tokio::main]
async fn main() {
    let args = AppArgs::parse();

    log4rs::init_file(args.log4rs_path, Default::default()).unwrap();

    let raw_config = std::fs::File::open(args.config_path).expect("Cannot load configuration file");
    let config: AppConfig = serde_json::from_reader(raw_config).expect("Invalid configuration file");

    let (tx_submission_agent, tx_submission_channel) = TxSubmissionAgent::<CONWAY_ERA_ID, Transaction>::new(
        config.node.clone(),
        config.tx_submission_buffer_size,
    )
    .await
    .expect("LocalTxSubmission initialization failed");
    let tx_submission_stream = tx_submission_agent.stream();

    let ip_addr = IpAddr::from_str(&*args.host).expect("Invalid host address");
    let bind_addr = SocketAddr::new(ip_addr, args.port);
    let server = build_api_server(config.limits, tx_submission_channel, bind_addr)
        .await
        .expect("Error setting up api server")
        .map(|r| r.unwrap());

    let processes = FuturesUnordered::new();
    let tx_submission_handle = tokio::spawn(run_stream(tx_submission_stream));
    processes.push(tx_submission_handle);
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
#[command(name = "splash-mempool-proxy")]
#[command(author = "Spectrum Labs")]
#[command(version = "1.0.0")]
#[command(about = "Splash Mempool Proxy", long_about = None)]
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
