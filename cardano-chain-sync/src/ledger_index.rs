use async_trait::async_trait;

use crate::client::Point;

#[async_trait]
pub trait LedgerCache<Block> {
    async fn set_tip(&self, point: Point);
    async fn get_tip(&self) -> Option<Point>;
    async fn put_block(&self, point: Point, block: Block);
    async fn get_block(&self, point: Point) -> Option<Block>;
    async fn exists(&self, point: Point) -> bool;
    async fn delete(&self, point: Point) -> bool;
}
