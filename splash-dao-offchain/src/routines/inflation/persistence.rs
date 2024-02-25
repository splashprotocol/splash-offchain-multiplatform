use std::marker::PhantomData;

use bloom_offchain::execution_engine::bundled::Bundled;

use crate::entities::inflation_box::InflationBox;
use crate::entities::poll_factory::PollFactory;
use crate::entities::weighting_poll::WeightingPoll;
use crate::routines::inflation::{proof, RoutineStateMarker};

#[async_trait::async_trait]
pub trait InflationStateRead<Out> {
    async fn state_marker(&self) -> RoutineStateMarker;
    async fn inflation_box(&self) -> Bundled<InflationBox, Out>;
    async fn poll_factory(&self) -> Bundled<PollFactory, Out>;
    async fn weighting_poll(&self, poll_exists: proof::PollExists) -> Bundled<WeightingPoll, Out>;
}

#[async_trait::async_trait]
pub trait RoutineStateWrite {
    async fn set_state(&self, state: RoutineStateMarker);
}

pub struct InflationRoutinePersistence<Out> {
    pd: PhantomData<Out>,
}

#[async_trait::async_trait]
impl<Out: Send + Sync> RoutineStateWrite for InflationRoutinePersistence<Out> {
    async fn set_state(&self, state: RoutineStateMarker) {
        todo!()
    }
}
