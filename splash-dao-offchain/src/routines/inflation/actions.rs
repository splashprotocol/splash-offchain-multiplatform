use bloom_offchain::execution_engine::bundled::Bundled;
use spectrum_offchain::data::unique_entity::{Predicted, Traced};

use crate::entities::offchain::voting_order::VotingOrder;
use crate::entities::onchain::inflation_box::InflationBox;
use crate::entities::onchain::poll_factory::PollFactory;
use crate::entities::onchain::smart_farm::SmartFarm;
use crate::entities::onchain::voting_escrow::VotingEscrow;
use crate::entities::onchain::weighting_poll::WeightingPoll;
use crate::time::ProtocolEpoch;

#[async_trait::async_trait]
pub trait InflationActions<Bearer> {
    async fn create_wpoll(
        &self,
        inflation_box: Bundled<InflationBox, Bearer>,
        factory: Bundled<PollFactory, Bearer>,
        epoch: ProtocolEpoch,
    ) -> (
        Traced<Predicted<Bundled<InflationBox, Bearer>>>,
        Traced<Predicted<Bundled<PollFactory, Bearer>>>,
        Traced<Predicted<Bundled<WeightingPoll, Bearer>>>,
    );
    async fn eliminate_wpoll(&self, weighting_poll: Bundled<WeightingPoll, Bearer>);
    async fn execute_order(
        &self,
        weighting_poll: Bundled<WeightingPoll, Bearer>,
        order: (VotingOrder, Bundled<VotingEscrow, Bearer>),
    ) -> (
        Traced<Predicted<Bundled<WeightingPoll, Bearer>>>,
        Traced<Predicted<Bundled<VotingEscrow, Bearer>>>,
    );
    async fn distribute_inflation(
        &self,
        weighting_poll: Bundled<WeightingPoll, Bearer>,
        farm: Bundled<SmartFarm, Bearer>,
    ) -> (
        Traced<Predicted<Bundled<WeightingPoll, Bearer>>>,
        Traced<Predicted<Bundled<SmartFarm, Bearer>>>,
    );
}
