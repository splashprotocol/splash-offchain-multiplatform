use crate::handler::unapply_tx;
use crate::index::UtxoIndex;
use bloom_offchain_cardano::event_sink::tx_view::TxViewMut;
use cardano_chain_sync::cache::{LedgerCache, LinkedBlock};
use cardano_chain_sync::client::Point;
use cardano_chain_sync::event_source::unpack_valid_transactions_multi_era;
use cml_chain::Deserialize;
use cml_multi_era::MultiEraBlock;
use log::{error, info, warn};
use std::sync::Arc;
use tokio::sync::Mutex;

/// Rollback both databases by walking backwards N blocks.
pub async fn rollback_blocks<Index, Cache>(
    utxo_index: &Index,
    chain_sync_cache: Arc<Mutex<Cache>>,
    current_point: Point,
    rollback_blocks: u64,
) -> Result<Point, String>
where
    Index: UtxoIndex + Send + Sync,
    Cache: LedgerCache + Send,
{
    let current_slot = current_slot_val(current_point);
    let target_slot = current_slot.saturating_sub(rollback_blocks);

    info!(
        "Starting rollback from slot {} to slot {} ({} blocks)",
        current_slot,
        target_slot,
        rollback_blocks.min(current_slot)
    );

    let mut current = current_point;
    let mut rollback_target = current_point;
    let mut rolled_back_any = false;

    while current_slot_val(current) > target_slot {
        let linked_block = match chain_sync_cache.lock().await.get_block(current).await {
            Some(block) => block,
            None => {
                if !rolled_back_any {
                    error!(
                        "Cannot rollback: tip at slot {} not found in cache",
                        current_slot_val(current)
                    );
                    error!("Possible causes: slot <= disable_rollbacks_until, or cache cleared");
                    return Err(format!(
                        "First block {:?} not in cache - cannot rollback",
                        current
                    ));
                } else {
                    warn!("Block not found in cache: {:?} - stopping rollback", current);
                    break;
                }
            }
        };

        let multi_era_block = MultiEraBlock::from_cbor_bytes(&linked_block.0).map_err(|e| {
            format!(
                "Failed to decode block at slot {}: {}",
                current_slot_val(current),
                e
            )
        })?;

        let valid_txs = unpack_valid_transactions_multi_era(multi_era_block);

        for (tx, _slot, _block_number, _block_hash) in valid_txs.into_iter().rev() {
            let tx_view = TxViewMut::from(tx);
            unapply_tx(utxo_index, tx_view).await;
        }

        chain_sync_cache.lock().await.delete(current).await;
        rolled_back_any = true;
        current = linked_block.1;
        rollback_target = current;
    }

    info!("Updating cache tip to {:?}", rollback_target);
    chain_sync_cache.lock().await.set_tip(rollback_target).await;

    Ok(rollback_target)
}

fn current_slot_val(point: Point) -> u64 {
    match point {
        Point::Origin => 0,
        Point::Specific(slot, _) => slot,
    }
}
