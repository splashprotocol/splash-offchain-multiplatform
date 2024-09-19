use async_trait::async_trait;
use cml_chain::Coin;
use nonempty::NonEmpty;
use spectrum_offchain::data::event::{Confirmed, Predicted};

use crate::entities::onchain::funding_box::{FundingBox, FundingBoxId};

#[async_trait]
pub trait FundingRepo {
    /// Collect funding boxes that cover the specified `target`.
    async fn collect(&mut self, target: Coin) -> Result<NonEmpty<FundingBox>, ()>;
    async fn put_confirmed(&mut self, df: Confirmed<FundingBox>);
    async fn put_predicted(&mut self, df: Predicted<FundingBox>);
    async fn remove(&mut self, fid: FundingBoxId);
}
