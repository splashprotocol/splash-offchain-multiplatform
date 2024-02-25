use async_trait::async_trait;

pub type NetworkTime = u64;

#[async_trait]
pub trait NetworkTimeProvider {
    async fn network_time(&self) -> NetworkTime;
}
