use bloom_offchain::execution_engine::bundled::Bundled;

use crate::entities::inflation_box::InflationBox;
use crate::entities::poll_factory::PollFactory;
use crate::entities::weighting_poll::WeightingPoll;

pub enum InflationEvent<Out> {
    InflationBoxUpdated(Bundled<InflationBox, Out>),
    PollFactoryUpdated(Bundled<PollFactory, Out>),
    NewWeightingPoll(Bundled<WeightingPoll, Out>),
}
