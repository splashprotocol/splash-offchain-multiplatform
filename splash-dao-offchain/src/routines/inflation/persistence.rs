use std::marker::PhantomData;

use log4rs::append::Append;

use bloom_offchain::execution_engine::bundled::Bundled;

use crate::entities::offchain::voting_order::VotingOrder;
use crate::entities::onchain::inflation_box::InflationBox;
use crate::entities::onchain::poll_factory::PollFactory;
use crate::entities::onchain::smart_farm::SmartFarm;
use crate::entities::onchain::voting_escrow::VotingEscrow;
use crate::entities::onchain::weighting_poll::{DistributionOngoing, WeightingOngoing, WeightingPoll};

#[async_trait::async_trait]
pub trait InflationStateRead<Out, TxId> {
    async fn inflation_box(&self) -> Bundled<InflationBox, Out>;
    async fn poll_factory(&self) -> Bundled<PollFactory, Out>;
    async fn weighting_poll(&self) -> Option<Bundled<WeightingPoll, Out>>;
    async fn pending_tx(&self) -> Option<TxId>;
    async fn next_order(
        &self,
        weighting_ongoing: WeightingOngoing,
    ) -> Option<(VotingOrder, Bundled<VotingEscrow, Out>)>;
    async fn next_farm(&self, distribution_ongoing: DistributionOngoing) -> Bundled<SmartFarm, Out>;
}

#[async_trait::async_trait]
pub trait InflationStateWrite<TxId> {}

pub struct InflationRoutinePersistence<Out> {
    pd: PhantomData<Out>,
}
