use bloom_offchain::execution_engine::bundled::Bundled;

use crate::entities::offchain::voting_order::{VotingOrder, VotingOrderId};
use crate::entities::onchain::poll_factory::PollFactory;
use crate::entities::onchain::smart_farm::SmartFarm;
use crate::entities::onchain::voting_escrow::VotingEscrow;
use crate::entities::onchain::weighting_poll::WeightingPoll;
use crate::time::ProtocolEpoch;
use crate::FarmId;

#[async_trait::async_trait]
pub trait InflationActions<Out, TxId> {
    async fn create_wpoll(&self, factory: Bundled<PollFactory, Out>, epoch: ProtocolEpoch) -> Option<TxId>;
    async fn eliminate_wpoll(&self, weighting_poll: Bundled<WeightingPoll, Out>) -> Option<TxId>;
    async fn execute_order(
        &self,
        weighting_poll: Bundled<WeightingPoll, Out>,
        order: (VotingOrder, Bundled<VotingEscrow, Out>),
    ) -> Result<VotingOrderId, VotingOrderId>;
    async fn distribute_inflation(
        &self,
        weighting_poll: Bundled<WeightingPoll, Out>,
        farm: Bundled<SmartFarm, Out>,
    ) -> FarmId;
}
