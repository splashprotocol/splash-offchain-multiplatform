use std::marker::PhantomData;

use log4rs::append::Append;

use bloom_offchain::execution_engine::bundled::Bundled;

use crate::entities::inflation_box::InflationBox;
use crate::entities::poll_factory::PollFactory;
use crate::entities::weighting_poll::WeightingPoll;

#[async_trait::async_trait]
pub trait InflationStateRead<Out, TxId> {
    async fn inflation_box(&self) -> Bundled<InflationBox, Out>;
    async fn poll_factory(&self) -> Bundled<PollFactory, Out>;
    async fn weighting_poll(&self) -> Option<Bundled<WeightingPoll, Out>>;
    async fn pending_tx(&self) -> Option<TxId>;
}

#[async_trait::async_trait]
pub trait InflationStateWrite {}

pub struct InflationRoutinePersistence<Out> {
    pd: PhantomData<Out>,
}

#[async_trait::async_trait]
impl<Out: Send + Sync> InflationStateWrite for InflationRoutinePersistence<Out> {}
