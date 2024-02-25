use bloom_offchain::execution_engine::bundled::Bundled;
use spectrum_offchain::data::Has;

use crate::coin::CoinSeedConfig;
use crate::entities::inflation_box::InflationBox;
use crate::entities::poll_factory::PollFactory;
use crate::entities::weighting_poll::WeightingPoll;
use crate::epoch_start;
use crate::network_time::{NetworkTime, NetworkTimeProvider};
use crate::routine::{postpone, transit, Attempt, StateRead};
use crate::routines::inflation::persistence::InflationStateRead;

mod persistence;

mod proof {
    use std::marker::PhantomData;

    #[derive(Copy, Clone, Debug)]
    /// Proof that poll exists.
    pub struct PollExists(PhantomData<()>);
}

#[derive(Copy, Clone, Debug)]
pub enum RoutineStateMarker {
    /// Too early to do anything.
    Idle,
    /// It's time to create a new WP for epoch `e`
    /// and pour it with epochly emission.
    PendingCreatePoll,
    /// Weighting in progress, applying votes from GT holders.
    WeightingInProgress(proof::PollExists),
    /// Weighting ended. Time to distribute inflation to farms pro-rata.
    DistributionInProgress(proof::PollExists),
    /// Inflation is distributed, time to eliminate the poll.
    PendingEliminatePoll(proof::PollExists),
}

pub enum RoutineState<Out> {
    /// Too early to do anything.
    Idle(IdleSt),
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

#[async_trait::async_trait]
impl<Out, P, T, C> StateRead<RoutineState<Out>, T, C> for P
where
    Out: Send + Sync,
    P: InflationStateRead<Out> + Send + Sync,
    C: Has<CoinSeedConfig> + Send + Sync + 'static,
    T: NetworkTimeProvider + Send + Sync,
{
    async fn state(&self, ntp: &T, ctx: C) -> RoutineState<Out> {
        match self.state_marker().await {
            RoutineStateMarker::Idle => RoutineState::Idle(IdleSt {
                until: epoch_start(
                    ctx.get().zeroth_epoch_start,
                    self.inflation_box().await.0.last_processed_epoch + 1,
                ),
                current_time: ntp.network_time().await,
            }),
            RoutineStateMarker::PendingCreatePoll => RoutineState::PendingCreatePoll(PendingCreatePoll {
                inflation_box: self.inflation_box().await,
                poll_factory: self.poll_factory().await,
            }),
            RoutineStateMarker::WeightingInProgress(poll_exists) => {
                RoutineState::WeightingInProgress(WeightingInProgress {
                    weighting_poll: self.weighting_poll(poll_exists).await,
                })
            }
            RoutineStateMarker::DistributionInProgress(poll_exists) => {
                RoutineState::DistributionInProgress(DistributionInProgress {
                    weighting_poll: self.weighting_poll(poll_exists).await,
                })
            }
            RoutineStateMarker::PendingEliminatePoll(poll_exists) => {
                RoutineState::PendingEliminatePoll(PendingEliminatePoll {
                    weighting_poll: self.weighting_poll(poll_exists).await,
                })
            }
        }
    }
}

pub enum StateTransition {
    ToPendingCreatePoll,
}

pub struct IdleSt {
    until: NetworkTime,
    current_time: NetworkTime,
}

impl IdleSt {
    fn can_start(&self) -> bool {
        self.until <= self.current_time
    }
}

#[async_trait::async_trait]
impl Attempt<StateTransition> for IdleSt {
    async fn attempt(self) -> Result<Option<StateTransition>, NetworkTime> {
        if self.can_start() {
            transit(StateTransition::ToPendingCreatePoll)
        } else {
            postpone(self.until)
        }
    }
}

pub struct PendingCreatePoll<Out> {
    inflation_box: Bundled<InflationBox, Out>,
    poll_factory: Bundled<PollFactory, Out>,
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

pub enum Event<Out> {
    InflationBoxUpdated(Bundled<InflationBox, Out>),
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use futures::channel::mpsc;
    use futures::StreamExt;
    use tokio::sync::Mutex;

    use bloom_offchain::execution_engine::bundled::Bundled;

    use crate::entities::inflation_box::InflationBox;
    use crate::entities::poll_factory::PollFactory;
    use crate::entities::weighting_poll::WeightingPoll;
    use crate::network_time::{NetworkTime, NetworkTimeProvider};
    use crate::routines::inflation::persistence::InflationStateRead;
    use crate::routines::inflation::proof::PollExists;
    use crate::routines::inflation::RoutineStateMarker;

    struct NTP(Arc<Mutex<mpsc::Receiver<NetworkTime>>>);
    #[async_trait::async_trait]
    impl NetworkTimeProvider for NTP {
        async fn network_time(&self) -> NetworkTime {
            self.0.lock().await.select_next_some().await
        }
    }

    struct DB<Out> {
        state: RoutineStateMarker,
        best_inflation_box: Bundled<InflationBox, Out>,
    }

    #[async_trait::async_trait]
    impl<Out: Send + Sync + Clone> InflationStateRead<Out> for Arc<Mutex<DB<Out>>> {
        async fn state_marker(&self) -> RoutineStateMarker {
            self.lock().await.state
        }

        async fn inflation_box(&self) -> Bundled<InflationBox, Out> {
            self.lock().await.best_inflation_box.clone()
        }

        async fn poll_factory(&self) -> Bundled<PollFactory, Out> {
            todo!()
        }

        async fn weighting_poll(&self, poll_exists: PollExists) -> Bundled<WeightingPoll, Out> {
            todo!()
        }
    }

    #[test]
    fn foo() {}
}
