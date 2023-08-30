use std::path::Path;

use pallas_network::miniprotocols::Point;

use cardano_chain_sync::client::{ChainSyncClient, ChainSyncConf};

#[tokio::main]
async fn main() {
    let chain_sync_conf = ChainSyncConf {
        path: Path::new("/var/lib/docker/volumes/cardano_node-ipc/_data/node.socket"),
        magic: 1,
        starting_point: Point::Origin,
    };
    let mut chain_sync = ChainSyncClient::init(chain_sync_conf).await.expect("ChainSync initialization wasn't successful");
    while let Some(x) = chain_sync.try_pull_next().await {
        serde_json::to_string_pretty(&x)
    }
}