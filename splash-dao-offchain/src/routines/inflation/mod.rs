use bloom_offchain::execution_engine::bundled::Bundled;
use spectrum_offchain::data::Has;

use crate::entities::inflation_box::InflationBox;
use crate::entities::poll_factory::PollFactory;
use crate::entities::weighting_poll::WeightingPoll;
use crate::GenesisEpochStartTime;
use crate::routine::{Attempt, postpone, StateRead, transit};
use crate::routines::inflation::actions::InflationActions;
use crate::routines::inflation::persistence::InflationStateRead;
use crate::routines::inflation::transitions::StateTransition;
use crate::time::{NetworkTime, NetworkTimeProvider};

mod actions;
mod events;
mod persistence;
mod transitions;

pub struct Config {
    pub genesis_epoch_start: GenesisEpochStartTime,
}

pub struct Behaviour<Actions, TimeProv, P> {
    actions: Actions,
    ntp: TimeProv,
    persistence: P,
    conf: Config,
}

impl<Actions, TimeProv, P> Behaviour<Actions, TimeProv, P> {
    fn in_state<S>(&self, state: S) -> BehaviourInState<Actions, TimeProv, S>
    where
        Actions: Clone,
        TimeProv: Clone,
    {
        BehaviourInState {
            actions: self.actions.clone(),
            ntp: self.ntp.clone(),
            state,
        }
    }
}

#[async_trait::async_trait]
impl<Out, P, TxId, Actions, TimeProv> StateRead<BehaviourInState<Actions, TimeProv, RoutineState<Out, TxId>>>
    for Behaviour<Actions, TimeProv, P>
where
    Out: Send + Sync,
    P: InflationStateRead<Out, TxId> + Send + Sync,
    Actions: Clone + Send + Sync,
    TimeProv: NetworkTimeProvider + Clone + Send + Sync,
    TxId: Send + Sync,
{
    async fn read_state(&self) -> BehaviourInState<Actions, TimeProv, RoutineState<Out, TxId>> {
        let now = self.ntp.network_time().await;
        self.in_state(match self.persistence.weighting_poll().await {
            Some(wp) if wp.0.weighting_open(now, self.conf.genesis_epoch_start) => {
                RoutineState::WeightingInProgress(WeightingInProgress { weighting_poll: wp })
            }
            Some(wp) if !wp.0.distribution_finished() => {
                RoutineState::DistributionInProgress(DistributionInProgress { weighting_poll: wp })
            }
            Some(wp) => RoutineState::PendingEliminatePoll(PendingEliminatePoll { weighting_poll: wp }),
            None => RoutineState::PendingCreatePoll(PendingCreatePoll {
                inflation_box: self.persistence.inflation_box().await,
                poll_factory: self.persistence.poll_factory().await,
                pending_attempt: self.persistence.pending_tx().await,
            }),
        })
    }
}

pub struct BehaviourInState<Actions, TimeProv, S> {
    actions: Actions,
    ntp: TimeProv,
    state: S,
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
    PendingEliminatePoll(PendingEliminatePoll<Out>),
}

pub struct PendingCreatePoll<Out, TxId> {
    inflation_box: Bundled<InflationBox, Out>,
    poll_factory: Bundled<PollFactory, Out>,
    pending_attempt: Option<TxId>,
}

const WAIT_CONFIRMATION: NetworkTime = 60 * 1000;

#[async_trait::async_trait]
impl<T, Out, TxId, Actions, TimeProv> Attempt<T, StateTransition<TxId>>
    for BehaviourInState<Actions, TimeProv, PendingCreatePoll<Out, TxId>>
where
    T: NetworkTimeProvider + Send + Sync,
    Out: Send + Sync,
    TxId: Send + Sync,
    TimeProv: Send + Sync,
    Actions: InflationActions<Out, TxId> + Send + Sync,
{
    async fn attempt(self) -> Result<Option<StateTransition<TxId>>, NetworkTime> {
        let state = self.state;
        if state.pending_attempt.is_some() {
            postpone(WAIT_CONFIRMATION)
        } else {
            let pending_epoch = state.poll_factory.0.next_epoch();
            let tx_id = self.actions.create_wpoll(state.poll_factory, pending_epoch).await;
            transit(StateTransition::PendingCreatePoll(tx_id))
        }
    }
}

pub struct WeightingInProgress<Out> {
    weighting_poll: Bundled<WeightingPoll, Out>,
}

pub struct DistributionInProgress<Out> {
    weighting_poll: Bundled<WeightingPoll, Out>,
}

pub struct PendingEliminatePoll<Out> {
    weighting_poll: Bundled<WeightingPoll, Out>,
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use futures::channel::mpsc;
    use tokio::sync::Mutex;

    use bloom_offchain::execution_engine::bundled::Bundled;

    use crate::entities::inflation_box::InflationBox;
    use crate::routines::inflation::persistence::InflationStateRead;
    use crate::time::NetworkTime;

    struct NTP(Arc<Mutex<mpsc::Receiver<NetworkTime>>>);

    struct DB<Out> {
        best_inflation_box: Bundled<InflationBox, Out>,
    }

    #[test]
    fn foo() {}
}
