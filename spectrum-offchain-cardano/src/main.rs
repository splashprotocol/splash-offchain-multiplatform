use std::path::Path;

use pallas_network::miniprotocols::Point;

use cardano_chain_sync::client::{ChainSyncClient, ChainSyncConf};

#[tokio::main]
async fn main() {
    let chain_sync_conf = ChainSyncConf {
        path: Path::new("/var/lib/docker/volumes/cardano_node-ipc/_data/node.socket"),
        magic: 1,
        starting_point: Point::Specific(37792291, hex::decode("516771c5f7bdb225a704afb67b0a31d86af8ae7cf747b65f7f5930dcd7381f48").unwrap()),
    };
    let mut chain_sync = ChainSyncClient::init(chain_sync_conf)
        .await
        .expect("ChainSync initialization wasn't successful");
    while let Some(blk) = chain_sync.try_pull_next().await {
        match blk {
            cardano_chain_sync::model::ChainUpgrade::RollBackward(_) => {
                println!("Rollback")
            }
            cardano_chain_sync::model::ChainUpgrade::RollForward(blk) => {
                println!("Upgrade")
            }
        }
    }
}
