use cardano_chain_sync::client::Point;
use spectrum_cardano_lib::NetworkId;
use spectrum_offchain_cardano::node::NodeConfig;

#[derive(Clone, Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppConfig {
    pub chain_sync: ChainSyncConfig,
    pub node: NodeConfig,
    pub network_id: NetworkId,
    pub tracked_limit_order_script_hashes: Vec<String>,
    pub index_db_path: String,
    pub http: HttpConfig,
}

#[derive(Clone, Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChainSyncConfig {
    pub starting_point: Point,
    pub disable_rollbacks_until: u64,
    pub db_path: String,
}

#[derive(Clone, Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HttpConfig {
    pub host: String,
    pub port: u16,
}

impl AppConfig {
    pub fn validate(&self) -> Result<(), String> {
        if self.chain_sync.db_path.is_empty() {
            return Err("chainSync.dbPath must not be empty".to_string());
        }
        if self.index_db_path.is_empty() {
            return Err("indexDbPath must not be empty".to_string());
        }
        if self.node.path.as_str().is_empty() {
            return Err("node.path must not be empty".to_string());
        }
        if self.tracked_limit_order_script_hashes.is_empty() {
            return Err("trackedLimitOrderScriptHashes must not be empty".to_string());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_requires_index_db_path() {
        let config = AppConfig {
            chain_sync: ChainSyncConfig {
                starting_point: Point::Origin,
                disable_rollbacks_until: 0,
                db_path: "chain.rocksdb".to_string(),
            },
            node: NodeConfig {
                path: "node.socket".into(),
                magic: 1,
            },
            network_id: NetworkId::PREPROD,
            tracked_limit_order_script_hashes: vec!["limit-order-script-hash".to_string()],
            index_db_path: String::new(),
            http: HttpConfig {
                host: "127.0.0.1".to_string(),
                port: 9030,
            },
        };

        assert_eq!(
            config.validate(),
            Err("indexDbPath must not be empty".to_string())
        );
    }

    #[test]
    fn config_requires_tracked_limit_order_script_hashes() {
        let config = AppConfig {
            chain_sync: ChainSyncConfig {
                starting_point: Point::Origin,
                disable_rollbacks_until: 0,
                db_path: "chain.rocksdb".to_string(),
            },
            node: NodeConfig {
                path: "node.socket".into(),
                magic: 1,
            },
            network_id: NetworkId::PREPROD,
            tracked_limit_order_script_hashes: vec![],
            index_db_path: "index.rocksdb".to_string(),
            http: HttpConfig {
                host: "127.0.0.1".to_string(),
                port: 9030,
            },
        };

        assert_eq!(
            config.validate(),
            Err("trackedLimitOrderScriptHashes must not be empty".to_string())
        );
    }
}
