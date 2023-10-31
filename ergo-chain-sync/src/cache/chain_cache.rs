use std::collections::HashMap;

use async_trait::async_trait;
use ergo_lib::ergo_chain_types::{BlockId, Digest32};

use crate::model::{Block, BlockRecord};

#[async_trait]
pub trait ChainCache {
    async fn append_block(&mut self, block: Block);
    async fn exists(&mut self, block_id: BlockId) -> bool;
    async fn get_best_block(&mut self) -> Option<BlockRecord>;
    async fn take_best_block(&mut self) -> Option<Block>;
}

pub struct InMemoryCache {
    blocks: HashMap<Digest32, Block>,
    best_block: Option<(BlockId, BlockId, u32)>,
}

impl InMemoryCache {
    pub fn new() -> Self {
        Self {
            blocks: HashMap::new(),
            best_block: None,
        }
    }
}

impl Default for InMemoryCache {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl ChainCache for InMemoryCache {
    async fn append_block(&mut self, block: Block) {
        let id = block.id;
        let parent_id = block.parent_id;
        let height = block.height;
        self.blocks.insert(id.0, block);
        self.best_block = Some((id, parent_id, height))
    }

    async fn exists(&mut self, block_id: BlockId) -> bool {
        self.blocks.contains_key(&block_id.0)
    }

    async fn get_best_block(&mut self) -> Option<BlockRecord> {
        self.best_block
            .as_ref()
            .map(|(id, _, h)| BlockRecord { id: *id, height: *h })
    }

    async fn take_best_block(&mut self) -> Option<Block> {
        if let Some((id, parent_id, _)) = self.best_block.take() {
            if let Some(parent_blk) = self.blocks.get(&parent_id.0) {
                self.best_block = Some((parent_blk.id, parent_blk.parent_id, parent_blk.height));
            }
            return self.blocks.remove(&id.0);
        }
        None
    }
}
