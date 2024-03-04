use std::marker::PhantomData;
use std::time::Duration;

use bloom_offchain::execution_engine::bundled::Bundled;
use spectrum_offchain::data::unique_entity::{AnyMod, Confirmed};

use crate::entities::offchain::voting_order::VotingOrder;
use crate::entities::onchain::inflation_box::InflationBox;
use crate::entities::onchain::poll_factory::PollFactory;
use crate::entities::onchain::smart_farm::SmartFarm;
use crate::entities::onchain::voting_escrow::VotingEscrow;
use crate::entities::onchain::weighting_poll::{
    DistributionOngoing, PollState, WeightingOngoing, WeightingPoll,
};
use crate::protocol_config::ProtocolConfig;
use crate::routine::{retry_in, RoutineBehaviour, ToRoutine};
use crate::routines::inflation::actions::InflationActions;
use crate::state_projection::{StateProjectionRead, StateProjectionWrite};
use crate::time::{NetworkTimeProvider, ProtocolEpoch};

mod actions;

pub struct Behaviour<IB, PF, WP, VE, SF, Time, Actions, Bearer> {
    inflation_box: IB,
    poll_factory: PF,
    weighting_poll: WP,
    voting_escrow: VE,
    smart_farm: SF,
    ntp: Time,
    actions: Actions,
    conf: ProtocolConfig,
    pd: PhantomData<Bearer>,
}

const DEF_DELAY: Duration = Duration::new(5, 0);

#[async_trait::async_trait]
impl<IB, PF, WP, VE, SF, Time, Actions, Bearer> RoutineBehaviour
    for Behaviour<IB, PF, WP, VE, SF, Time, Actions, Bearer>
where
    IB: StateProjectionRead<InflationBox, Bearer> + StateProjectionWrite<InflationBox, Bearer> + Send + Sync,
    PF: StateProjectionRead<PollFactory, Bearer> + StateProjectionWrite<PollFactory, Bearer> + Send + Sync,
    WP: StateProjectionRead<WeightingPoll, Bearer>
        + StateProjectionWrite<WeightingPoll, Bearer>
        + Send
        + Sync,
    VE: StateProjectionRead<VotingEscrow, Bearer> + StateProjectionWrite<VotingEscrow, Bearer> + Send + Sync,
    SF: StateProjectionRead<SmartFarm, Bearer> + StateProjectionWrite<SmartFarm, Bearer> + Send + Sync,
    Time: NetworkTimeProvider + Send + Sync,
    Actions: InflationActions<Bearer> + Send + Sync,
    Bearer: Send + Sync,
{
    async fn attempt(&self) -> Option<ToRoutine> {
        match self.read_state().await {
            RoutineState::Uninitialized => retry_in(DEF_DELAY),
            RoutineState::PendingCreatePoll(state) => self.try_create_wpoll(state).await,
            RoutineState::WeightingInProgress(state) => self.try_apply_votes(state).await,
            RoutineState::DistributionInProgress(state) => {
                self.try_distribute_inflation(state).await;
                None
            }
            RoutineState::PendingEliminatePoll(state) => self.try_eliminate_poll(state).await,
        }
    }
}

impl<IB, PF, WP, VE, SF, Time, Actions, Bearer> Behaviour<IB, PF, WP, VE, SF, Time, Actions, Bearer> {
    async fn inflation_box(&self) -> Option<AnyMod<Bundled<InflationBox, Bearer>>>
    where
        IB: StateProjectionRead<InflationBox, Bearer>,
    {
        self.inflation_box.read(self.conf.inflation_box_id).await
    }

    async fn poll_factory(&self) -> Option<AnyMod<Bundled<PollFactory, Bearer>>>
    where
        PF: StateProjectionRead<PollFactory, Bearer>,
    {
        self.poll_factory.read(self.conf.poll_factory_id).await
    }

    async fn weighting_poll(&self, epoch: ProtocolEpoch) -> Option<AnyMod<Bundled<WeightingPoll, Bearer>>>
    where
        WP: StateProjectionRead<WeightingPoll, Bearer>,
    {
        self.weighting_poll.read(self.conf.poll_id(epoch)).await
    }

    async fn next_order(
        &self,
        _stage: WeightingOngoing,
    ) -> Option<(VotingOrder, Bundled<VotingEscrow, Bearer>)> {
        todo!()
    }

    async fn next_farm(&self, _stage: DistributionOngoing) -> AnyMod<Bundled<SmartFarm, Bearer>> {
        todo!()
    }

    async fn read_state(&self) -> RoutineState<Bearer>
    where
        IB: StateProjectionRead<InflationBox, Bearer>,
        PF: StateProjectionRead<PollFactory, Bearer>,
        WP: StateProjectionRead<WeightingPoll, Bearer>,
        Time: NetworkTimeProvider,
    {
        if let (Some(inflation_box), Some(poll_factory)) =
            (self.inflation_box().await, self.poll_factory().await)
        {
            let genesis = self.conf.genesis_time;
            let now = self.ntp.network_time().await;
            let current_epoch = inflation_box.as_erased().0.active_epoch(genesis, now);
            match self.weighting_poll(current_epoch).await {
                None => RoutineState::PendingCreatePoll(PendingCreatePoll {
                    inflation_box,
                    poll_factory,
                    epoch: current_epoch,
                }),
                Some(wp) => match wp.as_erased().0.state(genesis, now) {
                    PollState::WeightingOngoing(st) => {
                        RoutineState::WeightingInProgress(WeightingInProgress {
                            weighting_poll: wp,
                            next_pending_order: self.next_order(st).await,
                        })
                    }
                    PollState::DistributionOngoing(st) => {
                        RoutineState::DistributionInProgress(DistributionInProgress {
                            weighting_poll: wp,
                            next_farm: self.next_farm(st).await,
                        })
                    }
                    PollState::PollExhausted(_) => {
                        RoutineState::PendingEliminatePoll(PendingEliminatePoll { weighting_poll: wp })
                    }
                },
            }
        } else {
            RoutineState::Uninitialized
        }
    }

    async fn try_create_wpoll(
        &self,
        PendingCreatePoll {
            inflation_box,
            poll_factory,
            epoch,
        }: PendingCreatePoll<Bearer>,
    ) -> Option<ToRoutine>
    where
        IB: StateProjectionWrite<InflationBox, Bearer>,
        PF: StateProjectionWrite<PollFactory, Bearer>,
        WP: StateProjectionWrite<WeightingPoll, Bearer>,
        Actions: InflationActions<Bearer>,
    {
        if let (AnyMod::Confirmed(inflation_box), AnyMod::Confirmed(factory)) = (inflation_box, poll_factory)
        {
            let (next_inflation_box, next_factory, next_wpoll) =
                self.actions.create_wpoll(inflation_box.0, factory.0, epoch).await;
            self.inflation_box.write(next_inflation_box).await;
            self.poll_factory.write(next_factory).await;
            self.weighting_poll.write(next_wpoll).await;
            return None;
        }
        retry_in(DEF_DELAY)
    }

    async fn try_apply_votes(
        &self,
        WeightingInProgress {
            weighting_poll,
            next_pending_order,
        }: WeightingInProgress<Bearer>,
    ) -> Option<ToRoutine>
    where
        WP: StateProjectionWrite<WeightingPoll, Bearer>,
        VE: StateProjectionWrite<VotingEscrow, Bearer>,
        Actions: InflationActions<Bearer>,
    {
        if let Some(next_order) = next_pending_order {
            let (next_wpoll, next_ve) = self
                .actions
                .execute_order(weighting_poll.erased(), next_order)
                .await;
            self.weighting_poll.write(next_wpoll).await;
            self.voting_escrow.write(next_ve).await;
            return None;
        }
        retry_in(DEF_DELAY)
    }

    async fn try_distribute_inflation(
        &self,
        DistributionInProgress {
            weighting_poll,
            next_farm,
        }: DistributionInProgress<Bearer>,
    ) where
        WP: StateProjectionWrite<WeightingPoll, Bearer>,
        SF: StateProjectionWrite<SmartFarm, Bearer>,
        Actions: InflationActions<Bearer>,
    {
        let (next_wpoll, next_sf) = self
            .actions
            .distribute_inflation(weighting_poll.erased(), next_farm.erased())
            .await;
        self.weighting_poll.write(next_wpoll).await;
        self.smart_farm.write(next_sf).await;
    }

    async fn try_eliminate_poll(
        &self,
        PendingEliminatePoll { weighting_poll }: PendingEliminatePoll<Bearer>,
    ) -> Option<ToRoutine>
    where
        Actions: InflationActions<Bearer>,
    {
        if let AnyMod::Confirmed(Confirmed(weighting_poll)) = weighting_poll {
            self.actions.eliminate_wpoll(weighting_poll).await;
            return None;
        }
        retry_in(DEF_DELAY)
    }
}

pub enum RoutineState<Out> {
    /// Protocol wasn't initialized yet.
    Uninitialized,
    /// It's time to create a new WP for epoch `e`
    /// and pour it with epochly emission.
    PendingCreatePoll(PendingCreatePoll<Out>),
    /// Weighting in progress, applying votes from GT holders.
    WeightingInProgress(WeightingInProgress<Out>),
    /// Weighting ended. Time to distribute inflation to farms pro-rata.
    DistributionInProgress(DistributionInProgress<Out>),
    /// Inflation is distributed, time to eliminate the poll.
    PendingEliminatePoll(PendingEliminatePoll<Out>),
}

pub struct PendingCreatePoll<Out> {
    inflation_box: AnyMod<Bundled<InflationBox, Out>>,
    poll_factory: AnyMod<Bundled<PollFactory, Out>>,
    epoch: ProtocolEpoch,
}

pub struct WeightingInProgress<Out> {
    weighting_poll: AnyMod<Bundled<WeightingPoll, Out>>,
    next_pending_order: Option<(VotingOrder, Bundled<VotingEscrow, Out>)>,
}

pub struct DistributionInProgress<Out> {
    weighting_poll: AnyMod<Bundled<WeightingPoll, Out>>,
    next_farm: AnyMod<Bundled<SmartFarm, Out>>,
}

pub struct PendingEliminatePoll<Out> {
    weighting_poll: AnyMod<Bundled<WeightingPoll, Out>>,
}
