use std::marker::PhantomData;
use std::time::Duration;

use futures::channel::mpsc;
use futures::{Stream, StreamExt};
use futures_timer::Delay;

use bloom_offchain::execution_engine::bundled::Bundled;

use crate::entities::offchain::voting_order::VotingOrder;
use crate::entities::onchain::inflation_box::InflationBox;
use crate::entities::onchain::poll_factory::PollFactory;
use crate::entities::onchain::smart_farm::SmartFarm;
use crate::entities::onchain::voting_escrow::VotingEscrow;
use crate::entities::onchain::weighting_poll::{PollState, WeightingPoll};
use crate::routine::{postpone, transit, ApplyEffect, Attempt, StateRead};
use crate::routines::inflation::actions::InflationActions;
use crate::routines::inflation::events::InflationEvent;
use crate::routines::inflation::persistence::{InflationStateRead, InflationStateWrite};
use crate::routines::inflation::transitions::InflationEffect;
use crate::time::NetworkTimeProvider;
use crate::tx_tracker::TransactionTracker;
use crate::GenesisEpochStartTime;

mod actions;
mod events;
mod persistence;
mod transitions;

pub struct Config {
    pub genesis_epoch_start: GenesisEpochStartTime,
}

pub struct Routine<Actions, TimeProv, P, TxTracker, Out, TxId> {
    actions: Actions,
    ntp: TimeProv,
    persistence: P,
    tracker: TxTracker,
    inbox: mpsc::Receiver<InflationEvent>,
    waker: Option<Delay>,
    conf: Config,
    pd: PhantomData<Out>,
    pd2: PhantomData<TxId>,
}

impl<Actions, TimeProv, P, TxTracker, Out, TxId> Routine<Actions, TimeProv, P, TxTracker, Out, TxId> {
    fn in_state<S>(&self, state: S) -> RoutineInState<Actions, TimeProv, S>
    where
        Actions: Clone,
        TimeProv: Clone,
    {
        RoutineInState {
            actions: self.actions.clone(),
            ntp: self.ntp.clone(),
            state,
        }
    }
    fn retry_in(&mut self, delay: Duration) {
        let _ = self.waker.insert(Delay::new(delay));
    }

    async fn apply_effect(&self, eff: InflationEffect<TxId>) {
        match eff {
            InflationEffect::AttemptCreatePoll(maybe_tx_id) => {
                if let Some(tx_id) = maybe_tx_id {
                    //self.tracker.track_status(tx_id, MAX_CONFIRM_TIME).await
                }
            }
            InflationEffect::AttemptEliminatePoll(_) => {}
            InflationEffect::AttemptOrderExec(_) => {}
            InflationEffect::InflationDistributedTo(_) => {}
        }
    }
}

pub const MAX_CONFIRM_TIME: Duration = Duration::new(60 * 10, 0);

pub struct RoutineInState<Actions, TimeProv, S> {
    actions: Actions,
    ntp: TimeProv,
    state: S,
}

impl<Actions, TimeProv, Out, TxId> RoutineInState<Actions, TimeProv, RoutineState<Out, TxId>> {
    async fn attempt(self) -> Result<Option<InflationEffect<TxId>>, Duration> {
        todo!()
    }
}

impl<Out, P, TxId, Actions, TimeProv, TxTracker> Routine<Actions, TimeProv, P, TxTracker, Out, TxId>
where
    Out: Send + Sync,
    P: InflationStateRead<Out, TxId> + Send + Sync,
    Actions: Clone + Send + Sync,
    TimeProv: NetworkTimeProvider + Clone + Send + Sync,
    TxId: Send + Sync,
{
    async fn read_state(&self) -> RoutineInState<Actions, TimeProv, RoutineState<Out, TxId>> {
        let now = self.ntp.network_time().await;
        self.in_state(match self.persistence.weighting_poll().await {
            None => RoutineState::PendingCreatePoll(PendingCreatePoll {
                inflation_box: self.persistence.inflation_box().await,
                poll_factory: self.persistence.poll_factory().await,
                pending_attempt: self.persistence.pending_tx().await,
            }),
            Some(wp) => match wp.0.state(now, self.conf.genesis_epoch_start) {
                PollState::WeightingOngoing(proof) => {
                    RoutineState::WeightingInProgress(WeightingInProgress {
                        weighting_poll: wp,
                        next_pending_order: self.persistence.next_order(proof).await,
                    })
                }
                PollState::DistributionOngoing(proof) => {
                    RoutineState::DistributionInProgress(DistributionInProgress {
                        weighting_poll: wp,
                        next_farm: self.persistence.next_farm(proof).await,
                    })
                }
                PollState::PollExhausted(_) => RoutineState::PendingEliminatePoll(PendingEliminatePoll {
                    weighting_poll: wp,
                    pending_attempt: self.persistence.pending_tx().await,
                }),
            },
        })
    }
}

pub enum RoutineState<Out, TxId> {
    /// It's time to create a new WP for epoch `e`
    /// and pour it with epochly emission.
    PendingCreatePoll(PendingCreatePoll<Out, TxId>),
    /// Weighting in progress, applying votes from GT holders.
    WeightingInProgress(WeightingInProgress<Out>),
    /// Weighting ended. Time to distribute inflation to farms pro-rata.
    DistributionInProgress(DistributionInProgress<Out>),
    /// Inflation is distributed, time to eliminate the poll.
    PendingEliminatePoll(PendingEliminatePoll<Out, TxId>),
}

pub struct PendingCreatePoll<Out, TxId> {
    inflation_box: Bundled<InflationBox, Out>,
    poll_factory: Bundled<PollFactory, Out>,
    pending_attempt: Option<TxId>,
}

const WAIT_CONFIRMATION: Duration = Duration::new(60 * 10, 0);
const WAIT_ORDER: Duration = Duration::new(60 * 10, 0);

#[async_trait::async_trait]
impl<Out, TxId, Actions, TimeProv> Attempt<InflationEffect<TxId>>
    for RoutineInState<Actions, TimeProv, PendingCreatePoll<Out, TxId>>
where
    Out: Send + Sync,
    TxId: Send + Sync,
    TimeProv: NetworkTimeProvider + Send + Sync,
    Actions: InflationActions<Out, TxId> + Send + Sync,
{
    async fn attempt(self) -> Result<Option<InflationEffect<TxId>>, Duration> {
        let state = self.state;
        if state.pending_attempt.is_some() {
            postpone(WAIT_CONFIRMATION)
        } else {
            let pending_epoch = state.poll_factory.0.next_epoch();
            let result = self.actions.create_wpoll(state.poll_factory, pending_epoch).await;
            transit(InflationEffect::AttemptCreatePoll(result))
        }
    }
}

pub struct WeightingInProgress<Out> {
    weighting_poll: Bundled<WeightingPoll, Out>,
    next_pending_order: Option<(VotingOrder, Bundled<VotingEscrow, Out>)>,
}

#[async_trait::async_trait]
impl<Out, TxId, Actions, TimeProv> Attempt<InflationEffect<TxId>>
    for RoutineInState<Actions, TimeProv, WeightingInProgress<Out>>
where
    Out: Send + Sync,
    TxId: Send + Sync,
    TimeProv: NetworkTimeProvider + Send + Sync,
    Actions: InflationActions<Out, TxId> + Send + Sync,
{
    async fn attempt(self) -> Result<Option<InflationEffect<TxId>>, Duration> {
        if let Some(order) = self.state.next_pending_order {
            let result = self.actions.execute_order(self.state.weighting_poll, order).await;
            transit(InflationEffect::AttemptOrderExec(result))
        } else {
            postpone(WAIT_ORDER)
        }
    }
}

pub struct DistributionInProgress<Out> {
    weighting_poll: Bundled<WeightingPoll, Out>,
    next_farm: Bundled<SmartFarm, Out>,
}

#[async_trait::async_trait]
impl<Out, TxId, Actions, TimeProv> Attempt<InflationEffect<TxId>>
    for RoutineInState<Actions, TimeProv, DistributionInProgress<Out>>
where
    Out: Send + Sync,
    TxId: Send + Sync,
    TimeProv: NetworkTimeProvider + Send + Sync,
    Actions: InflationActions<Out, TxId> + Send + Sync,
{
    async fn attempt(self) -> Result<Option<InflationEffect<TxId>>, Duration> {
        let farm_id = self
            .actions
            .distribute_inflation(self.state.weighting_poll, self.state.next_farm)
            .await;
        transit(InflationEffect::InflationDistributedTo(farm_id))
    }
}

pub struct PendingEliminatePoll<Out, TxId> {
    weighting_poll: Bundled<WeightingPoll, Out>,
    pending_attempt: Option<TxId>,
}

#[async_trait::async_trait]
impl<Out, TxId, Actions, TimeProv> Attempt<InflationEffect<TxId>>
    for RoutineInState<Actions, TimeProv, PendingEliminatePoll<Out, TxId>>
where
    Out: Send + Sync,
    TxId: Send + Sync,
    TimeProv: NetworkTimeProvider + Send + Sync,
    Actions: InflationActions<Out, TxId> + Send + Sync,
{
    async fn attempt(self) -> Result<Option<InflationEffect<TxId>>, Duration> {
        let state = self.state;
        if state.pending_attempt.is_some() {
            postpone(WAIT_CONFIRMATION)
        } else {
            let result = self.actions.eliminate_wpoll(state.weighting_poll).await;
            transit(InflationEffect::AttemptEliminatePoll(result))
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use futures::channel::mpsc;
    use tokio::sync::Mutex;

    use bloom_offchain::execution_engine::bundled::Bundled;

    use crate::entities::offchain::voting_order::VotingOrder;
    use crate::entities::onchain::inflation_box::InflationBox;
    use crate::entities::onchain::poll_factory::PollFactory;
    use crate::entities::onchain::voting_escrow::VotingEscrow;
    use crate::entities::onchain::weighting_poll::WeightingPoll;
    use crate::routines::inflation::persistence::InflationStateRead;
    use crate::time::NetworkTime;
    use crate::FarmId;

    struct NTP(Arc<Mutex<mpsc::Receiver<NetworkTime>>>);

    struct DB<Out> {
        best_inflation_box: Option<Bundled<InflationBox, Out>>,
        factory: Option<Bundled<PollFactory, Out>>,
        weighting_poll: Option<Bundled<WeightingPoll, Out>>,
        orders: Vec<(VotingOrder, Bundled<VotingEscrow, Out>)>,
        farms_processed_in_this_epoch: Vec<FarmId>,
    }

    #[test]
    fn foo() {}

    #[test]
    fn foo2() {}
}
