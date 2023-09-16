#[async_trait::async_trait]
pub trait Network<Tx, Err> {
    async fn submit_tx(&mut self, tx: Tx) -> Result<(), Err>;
}
