#[derive(Debug, derive_more::Display)]
pub struct ClientError(pub String);

#[async_trait::async_trait]
pub trait Network<Tx> {
    async fn submit_tx(&self, tx: Tx) -> Result<(), ClientError>;
}
