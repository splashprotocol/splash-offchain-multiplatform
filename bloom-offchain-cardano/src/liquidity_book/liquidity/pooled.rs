use std::collections::HashMap;
use crate::liquidity_book::pool::Pool;

use crate::liquidity_book::types::{Price, SourceId};

pub trait PooledLiquidity<Pl> {
    fn best_price(&self) -> Price;
    fn try_pick<F>(&mut self, test: F) -> Option<Pl>
    where
        F: FnOnce(&Pl) -> bool;
}

pub trait PoolStore<Pl> {
    fn update_pool(&mut self, pool: Pool);
}

#[derive(Debug, Clone)]
pub struct InMemoryPooledLiquidity<Pl> {
    pools: HashMap<SourceId, Pl>,
}
