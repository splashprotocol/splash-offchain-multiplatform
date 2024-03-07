use cml_chain::transaction::TransactionOutput;

use bloom_offchain::execution_engine::bundled::Bundled;
use spectrum_cardano_lib::transaction::TransactionOutputExtension;
use spectrum_offchain::data::EntitySnapshot;
use spectrum_offchain::data::unique_entity::{Predicted, Traced};
use spectrum_offchain::ledger::IntoLedger;

use crate::assets::SPLASH_AC;
use crate::entities::offchain::voting_order::VotingOrder;
use crate::entities::onchain::inflation_box::InflationBox;
use crate::entities::onchain::poll_factory::{PollFactory, unsafe_update_factory_state};
use crate::entities::onchain::smart_farm::SmartFarm;
use crate::entities::onchain::voting_escrow::VotingEscrow;
use crate::entities::onchain::weighting_poll::WeightingPoll;

#[async_trait::async_trait]
pub trait InflationActions<Bearer> {
    async fn create_wpoll(
        &self,
        inflation_box: Bundled<InflationBox, Bearer>,
        factory: Bundled<PollFactory, Bearer>,
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
        farm_weight: u64,
    ) -> (
        Traced<Predicted<Bundled<WeightingPoll, Bearer>>>,
        Traced<Predicted<Bundled<SmartFarm, Bearer>>>,
    );
}

pub struct CardanoInflationActions<Ctx> {
    ctx: Ctx,
}

#[async_trait::async_trait]
impl<Ctx> InflationActions<TransactionOutput> for CardanoInflationActions<Ctx>
where
    Ctx: Send + Sync + Copy,
{
    async fn create_wpoll(
        &self,
        Bundled(inflation_box, inflation_box_in): Bundled<InflationBox, TransactionOutput>,
        Bundled(factory, factory_in): Bundled<PollFactory, TransactionOutput>,
    ) -> (
        Traced<Predicted<Bundled<InflationBox, TransactionOutput>>>,
        Traced<Predicted<Bundled<PollFactory, TransactionOutput>>>,
        Traced<Predicted<Bundled<WeightingPoll, TransactionOutput>>>,
    ) {
        let prev_ib_version = inflation_box.version();
        let (next_inflation_box, rate) = inflation_box.release_next_tranche();
        let mut inflation_box_out = inflation_box_in.clone();
        if let Some(data_mut) = inflation_box_out.data_mut() {
            unsafe_update_factory_state(data_mut, next_inflation_box.last_processed_epoch);
        }
        inflation_box_out.sub_asset(*SPLASH_AC, rate.untag());
        let prev_factory_version = factory.version();
        let (next_factory, fresh_wpoll) = factory.next_weighting_poll();
        let mut factory_out = factory_in.clone();
        if let Some(data_mut) = factory_out.data_mut() {
            unsafe_update_factory_state(data_mut, next_factory.last_poll_epoch);
        }
        let next_traced_ibox = Traced::new(
            Predicted(Bundled(next_inflation_box, inflation_box_out.clone())),
            Some(prev_ib_version),
        );
        let next_traced_factory = Traced::new(
            Predicted(Bundled(next_factory, factory_out.clone())),
            Some(prev_factory_version),
        );
        let wpoll_out = fresh_wpoll.clone().into_ledger(self.ctx);
        let fresh_wpoll = Traced::new(Predicted(Bundled(fresh_wpoll, wpoll_out.clone())), None);
        (next_traced_ibox, next_traced_factory, fresh_wpoll)
    }

    async fn eliminate_wpoll(&self, weighting_poll: Bundled<WeightingPoll, TransactionOutput>) {
        todo!()
    }

    async fn execute_order(
        &self,
        weighting_poll: Bundled<WeightingPoll, TransactionOutput>,
        order: (VotingOrder, Bundled<VotingEscrow, TransactionOutput>),
    ) -> (
        Traced<Predicted<Bundled<WeightingPoll, TransactionOutput>>>,
        Traced<Predicted<Bundled<VotingEscrow, TransactionOutput>>>,
    ) {
        todo!()
    }

    async fn distribute_inflation(
        &self,
        weighting_poll: Bundled<WeightingPoll, TransactionOutput>,
        farm: Bundled<SmartFarm, TransactionOutput>,
        farm_weight: u64,
    ) -> (
        Traced<Predicted<Bundled<WeightingPoll, TransactionOutput>>>,
        Traced<Predicted<Bundled<SmartFarm, TransactionOutput>>>,
    ) {
        todo!()
    }
}
