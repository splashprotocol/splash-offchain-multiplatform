use std::marker::PhantomData;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use bloom_offchain::execution_engine::bundled::Bundled;
use spectrum_cardano_lib::OutputRef;
use spectrum_offchain::backlog::ResilientBacklog;
use spectrum_offchain::data::unique_entity::{AnyMod, Confirmed};

use crate::entities::offchain::voting_order::VotingOrder;
use crate::entities::onchain::inflation_box::InflationBox;
use crate::entities::onchain::poll_factory::PollFactory;
use crate::entities::onchain::smart_farm::SmartFarm;
use crate::entities::onchain::voting_escrow::{VotingEscrow, VotingEscrowId};
use crate::entities::onchain::weighting_poll::{PollState, WeightingOngoing, WeightingPoll};
use crate::entities::Snapshot;
use crate::protocol_config::ProtocolConfig;
use crate::routine::{retry_in, RoutineBehaviour, ToRoutine};
use crate::routines::inflation::actions::InflationActions;
use crate::state_projection::{StateProjectionRead, StateProjectionWrite};
use crate::time::{NetworkTimeProvider, ProtocolEpoch};

pub mod actions;

pub struct Behaviour<IB, PF, WP, VE, SF, Backlog, Time, Actions, Bearer> {
    inflation_box: IB,
    poll_factory: PF,
    weighting_poll: WP,
    voting_escrow: VE,
    smart_farm: SF,
    backlog: Backlog,
    ntp: Time,
    actions: Actions,
    conf: ProtocolConfig,
    pd: PhantomData<Bearer>,
}

const DEF_DELAY: Duration = Duration::new(5, 0);

pub type InflationBoxSnapshot = Snapshot<InflationBox, OutputRef>;
pub type PollFactorySnapshot = Snapshot<PollFactory, OutputRef>;
pub type WeightingPollSnapshot = Snapshot<WeightingPoll, OutputRef>;
pub type VotingEscrowSnapshot = Snapshot<VotingEscrow, OutputRef>;

#[async_trait::async_trait]
impl<IB, PF, WP, VE, SF, Backlog, Time, Actions, Bearer> RoutineBehaviour
    for Behaviour<IB, PF, WP, VE, SF, Backlog, Time, Actions, Bearer>
where
    IB: StateProjectionRead<InflationBoxSnapshot, Bearer>
        + StateProjectionWrite<InflationBoxSnapshot, Bearer>
        + Send
        + Sync,
    PF: StateProjectionRead<PollFactorySnapshot, Bearer>
        + StateProjectionWrite<PollFactorySnapshot, Bearer>
        + Send
        + Sync,
    WP: StateProjectionRead<WeightingPollSnapshot, Bearer>
        + StateProjectionWrite<WeightingPollSnapshot, Bearer>
        + Send
        + Sync,
    VE: StateProjectionRead<VotingEscrowSnapshot, Bearer>
        + StateProjectionWrite<VotingEscrowSnapshot, Bearer>
        + Send
        + Sync,
    Backlog: ResilientBacklog<VotingOrder> + Send + Sync,
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

impl<IB, PF, WP, VE, SF, Backlog, Time, Actions, Bearer>
    Behaviour<IB, PF, WP, VE, SF, Backlog, Time, Actions, Bearer>
{
    async fn inflation_box(&self) -> Option<AnyMod<Bundled<InflationBoxSnapshot, Bearer>>>
    where
        IB: StateProjectionRead<InflationBoxSnapshot, Bearer>,
    {
        self.inflation_box.read(self.conf.inflation_box_id).await
    }

    async fn poll_factory(&self) -> Option<AnyMod<Bundled<PollFactorySnapshot, Bearer>>>
    where
        PF: StateProjectionRead<PollFactorySnapshot, Bearer>,
    {
        self.poll_factory.read(self.conf.poll_factory_id).await
    }

    async fn weighting_poll(
        &self,
        epoch: ProtocolEpoch,
    ) -> Option<AnyMod<Bundled<WeightingPollSnapshot, Bearer>>>
    where
        WP: StateProjectionRead<WeightingPollSnapshot, Bearer>,
    {
        self.weighting_poll.read(self.conf.poll_id(epoch)).await
    }

    async fn next_order(
        &self,
        _stage: WeightingOngoing,
    ) -> Option<(VotingOrder, Bundled<VotingEscrowSnapshot, Bearer>)>
    where
        Backlog: ResilientBacklog<VotingOrder>,
        VE: StateProjectionRead<VotingEscrowSnapshot, Bearer>,
    {
        if let Some(ord) = self.backlog.try_pop().await {
            self.voting_escrow
                .read(VotingEscrowId::from(ord.id))
                .await
                .map(|ve| (ord, ve.erased()))
        } else {
            None
        }
    }

    async fn read_state(&self) -> RoutineState<Bearer>
    where
        IB: StateProjectionRead<InflationBoxSnapshot, Bearer>,
        PF: StateProjectionRead<PollFactorySnapshot, Bearer>,
        WP: StateProjectionRead<WeightingPollSnapshot, Bearer>,
        SF: StateProjectionRead<SmartFarm, Bearer>,
        VE: StateProjectionRead<VotingEscrowSnapshot, Bearer>,
        Backlog: ResilientBacklog<VotingOrder>,
        Time: NetworkTimeProvider,
    {
        if let (Some(inflation_box), Some(poll_factory)) =
            (self.inflation_box().await, self.poll_factory().await)
        {
            let genesis = self.conf.genesis_time;
            let now = self.ntp.network_time().await;
            let current_epoch = inflation_box.as_erased().0.get().active_epoch(genesis, now);
            match self.weighting_poll(current_epoch).await {
                None => RoutineState::PendingCreatePoll(PendingCreatePoll {
                    inflation_box,
                    poll_factory,
                }),
                Some(wp) => match wp.as_erased().0.get().state(genesis, now) {
                    PollState::WeightingOngoing(st) => {
                        RoutineState::WeightingInProgress(WeightingInProgress {
                            weighting_poll: wp,
                            next_pending_order: self.next_order(st).await,
                        })
                    }
                    PollState::DistributionOngoing(next_farm) => {
                        RoutineState::DistributionInProgress(DistributionInProgress {
                            next_farm: self
                                .smart_farm
                                .read(next_farm.farm_id())
                                .await
                                .expect("State is inconsistent"),
                            weighting_poll: wp,
                            next_farm_weight: next_farm.farm_weight(),
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
        }: PendingCreatePoll<Bearer>,
    ) -> Option<ToRoutine>
    where
        IB: StateProjectionWrite<InflationBoxSnapshot, Bearer>,
        PF: StateProjectionWrite<PollFactorySnapshot, Bearer>,
        WP: StateProjectionWrite<WeightingPollSnapshot, Bearer>,
        Actions: InflationActions<Bearer>,
    {
        if let (AnyMod::Confirmed(inflation_box), AnyMod::Confirmed(factory)) = (inflation_box, poll_factory)
        {
            let current_posix_time = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs() as u32;
            let (signed_tx, next_inflation_box, next_factory, next_wpoll) = self
                .actions
                .create_wpoll(&self.conf, current_posix_time, inflation_box.0, factory.0)
                .await;
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
        WP: StateProjectionWrite<WeightingPollSnapshot, Bearer>,
        VE: StateProjectionWrite<VotingEscrowSnapshot, Bearer>,
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
            next_farm_weight,
        }: DistributionInProgress<Bearer>,
    ) where
        WP: StateProjectionWrite<WeightingPollSnapshot, Bearer>,
        SF: StateProjectionWrite<SmartFarm, Bearer>,
        Actions: InflationActions<Bearer>,
    {
        let (next_wpoll, next_sf) = self
            .actions
            .distribute_inflation(weighting_poll.erased(), next_farm.erased(), next_farm_weight)
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
        let current_posix_time = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs() as u32;
        if let AnyMod::Confirmed(Confirmed(weighting_poll)) = weighting_poll {
            let signed_tx = self
                .actions
                .eliminate_wpoll(&self.conf, current_posix_time, weighting_poll)
                .await;
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
    inflation_box: AnyMod<Bundled<InflationBoxSnapshot, Out>>,
    poll_factory: AnyMod<Bundled<PollFactorySnapshot, Out>>,
}

pub struct WeightingInProgress<Out> {
    weighting_poll: AnyMod<Bundled<WeightingPollSnapshot, Out>>,
    next_pending_order: Option<(VotingOrder, Bundled<VotingEscrowSnapshot, Out>)>,
}

pub struct DistributionInProgress<Out> {
    weighting_poll: AnyMod<Bundled<WeightingPollSnapshot, Out>>,
    next_farm: AnyMod<Bundled<SmartFarm, Out>>,
    next_farm_weight: u64,
}

pub struct PendingEliminatePoll<Out> {
    weighting_poll: AnyMod<Bundled<WeightingPollSnapshot, Out>>,
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use async_trait::async_trait;
    use tokio::sync::Mutex;

    use bloom_offchain::execution_engine::bundled::Bundled;
    use spectrum_offchain::data::unique_entity::{AnyMod, Predicted, Traced};
    use spectrum_offchain::data::{EntitySnapshot, Identifier};

    use crate::state_projection::{StateProjectionRead, StateProjectionWrite};

    struct StateProjection<T: EntitySnapshot, B>(Arc<Mutex<Option<AnyMod<Bundled<T, B>>>>>);
    #[async_trait]
    impl<T, B> StateProjectionWrite<T, B> for StateProjection<T, B>
    where
        T: EntitySnapshot + Send + Sync + Clone,
        T::Version: Send,
        B: Send + Sync + Clone,
    {
        async fn write(&self, entity: Traced<Predicted<Bundled<T, B>>>) {
            let _ = self.0.lock().await.insert(AnyMod::Predicted(entity));
        }
    }
    #[async_trait]
    impl<T, B> StateProjectionRead<T, B> for StateProjection<T, B>
    where
        T: EntitySnapshot + Send + Sync + Clone,
        T::Version: Send,
        B: Send + Sync + Clone,
    {
        async fn read<I>(&self, id: I) -> Option<AnyMod<Bundled<T, B>>>
        where
            I: Identifier<For = T> + Send,
        {
            self.0.lock().await.clone()
        }
    }
}
