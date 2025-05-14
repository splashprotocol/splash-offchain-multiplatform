use std::net::{IpAddr, SocketAddr};
use std::str::FromStr;
use clap::Parser;
use splash_distribution::validator::auth_requests_validator::AuthRequestsValidator;
use crate::config::validator_config::ValidatorConfig;
use crate::http_api::build_api_server;

pub mod http_api;
pub mod config;

#[tokio::main(flavor = "multi_thread", worker_threads = 8)]
async fn main() {

    let args = AppArgs::parse();
    let raw_config = std::fs::read_to_string(args.config_path).expect("Cannot load configuration file");
    let config: ValidatorConfig = serde_json::from_str(&raw_config).expect("Invalid configuration file");

    log4rs::init_file(args.log4rs_path, Default::default()).unwrap();

    let ip_addr = IpAddr::from_str(&*config.host).expect("Invalid host address");
    let bind_addr = SocketAddr::new(ip_addr, config.port);

    let validator: AuthRequestsValidator = AuthRequestsValidator::new(
        config.validator_private_key
    );

    let _ = build_api_server(validator, bind_addr)
        .await
        .expect("Error setting up api server")
        .map(|r| r.unwrap());
}

#[derive(Parser)]
#[command(name = "splash-validator-agent")]
#[command(author = "Spectrum Labs")]
#[command(version = "1.0.0")]
#[command(about = "Splash Validator Agent", long_about = None)]
struct AppArgs {
    /// Path to the JSON configuration file.
    #[arg(long, short)]
    config_path: String,
    /// Path to the log4rs YAML configuration file.
    #[arg(long, short)]
    log4rs_path: String,
}
