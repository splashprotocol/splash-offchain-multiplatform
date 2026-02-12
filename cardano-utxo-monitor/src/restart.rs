use cardano_chain_sync::cache::LedgerCache;
use cardano_chain_sync::client::Point;
use log::info;
use std::sync::Arc;
use tokio::sync::Mutex;

#[derive(Debug, Clone)]
pub enum RestartMode {
    FreshStart {
        from: Point,
    },
    RollbackAndResume {
        current_point: Point,
        rollback_blocks: u64,
    },
}

pub async fn determine_restart_mode<Cache>(
    chain_sync_cache: Arc<Mutex<Cache>>,
    starting_point: Point,
    auto_rollback_blocks: u64,
) -> RestartMode
where
    Cache: LedgerCache,
{
    info!("=== Determining Restart Mode ===");

    let cache_tip = chain_sync_cache.lock().await.get_tip().await;

    info!("Chain sync cache tip: {:?}", cache_tip);
    info!("Configured auto_rollback_blocks: {}", auto_rollback_blocks);

    match cache_tip {
        None => {
            info!("Empty state detected - starting fresh");
            RestartMode::FreshStart { from: starting_point }
        }

        Some(current_point) => {
            let current_slot = match current_point {
                Point::Origin => 0,
                Point::Specific(slot, _) => slot,
            };

            info!("Existing state detected at slot {}", current_slot);
            info!("Auto-rolling back {} blocks", auto_rollback_blocks);

            RestartMode::RollbackAndResume {
                current_point,
                rollback_blocks: auto_rollback_blocks,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cardano_chain_sync::cache::LedgerCacheRocksDB;
    use tempfile::TempDir;

    fn new_temp_cache() -> Arc<Mutex<LedgerCacheRocksDB>> {
        let dir = TempDir::new().unwrap();
        Arc::new(Mutex::new(LedgerCacheRocksDB::new(dir.path())))
    }

    #[tokio::test]
    async fn test_empty_cache_fresh_start() {
        let cache = new_temp_cache();
        let starting_point = Point::Origin;

        let mode = determine_restart_mode(cache, starting_point, 2160).await;

        assert!(matches!(mode, RestartMode::FreshStart { .. }));
    }

    #[tokio::test]
    async fn test_existing_cache_rollback() {
        let cache = new_temp_cache();
        let test_point = Point::Specific(1000, cml_crypto::BlockHeaderHash::from([0u8; 32]));
        cache.lock().await.set_tip(test_point).await;

        let mode = determine_restart_mode(cache, Point::Origin, 2160).await;

        assert!(matches!(
            mode,
            RestartMode::RollbackAndResume {
                current_point,
                rollback_blocks: 2160
            } if current_point == test_point
        ));
    }
}
