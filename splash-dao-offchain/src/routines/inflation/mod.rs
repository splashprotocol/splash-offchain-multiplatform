use std::marker::PhantomData;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use bloom_offchain::execution_engine::bundled::Bundled;
use cml_chain::transaction::Transaction;
use spectrum_cardano_lib::{AssetName, OutputRef};
use spectrum_offchain::backlog::ResilientBacklog;
use spectrum_offchain::data::unique_entity::{AnyMod, Confirmed, Traced};
use spectrum_offchain::network::Network;
use spectrum_offchain::tx_prover::TxProver;
use spectrum_offchain_cardano::prover::operator::OperatorProver;
use spectrum_offchain_cardano::tx_submission::TxRejected;

use crate::entities::offchain::voting_order::VotingOrder;
use crate::entities::onchain::inflation_box::{InflationBoxId, InflationBoxSnapshot};
use crate::entities::onchain::permission_manager::{PermManager, PermManagerId, PermManagerSnapshot};
use crate::entities::onchain::poll_factory::{PollFactory, PollFactoryId, PollFactorySnapshot};
use crate::entities::onchain::smart_farm::{FarmId, SmartFarm, SmartFarmSnapshot};
use crate::entities::onchain::voting_escrow::{VotingEscrow, VotingEscrowId, VotingEscrowSnapshot};
use crate::entities::onchain::weighting_poll::{
    PollState, WeightingOngoing, WeightingPoll, WeightingPollId, WeightingPollSnapshot,
};
use crate::entities::Snapshot;
use crate::protocol_config::ProtocolConfig;
use crate::routine::{retry_in, RoutineBehaviour, ToRoutine};
use crate::routines::inflation::actions::InflationActions;
use crate::state_projection::{StateProjectionRead, StateProjectionWrite};
use crate::time::{NetworkTimeProvider, ProtocolEpoch};

pub mod actions;

pub struct Behaviour<'a, IB, PF, WP, VE, SF, PM, Backlog, Time, Actions, Bearer, Net> {
    inflation_box: IB,
    poll_factory: PF,
    weighting_poll: WP,
    voting_escrow: VE,
    smart_farm: SF,
    perm_manager: PM,
    backlog: Backlog,
    ntp: Time,
    actions: Actions,
    conf: ProtocolConfig,
    pd: PhantomData<Bearer>,
    network: Net,
    prover: OperatorProver<'a>,
}

const DEF_DELAY: Duration = Duration::new(5, 0);

impl<'a, IB, PF, WP, VE, SF, PM, Backlog, Time, Actions, Bearer, Net>
    Behaviour<'a, IB, PF, WP, VE, SF, PM, Backlog, Time, Actions, Bearer, Net>
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
    SF: StateProjectionRead<SmartFarmSnapshot, Bearer>
        + StateProjectionWrite<SmartFarmSnapshot, Bearer>
        + Send
        + Sync,
    PM: StateProjectionRead<PermManagerSnapshot, Bearer>
        + StateProjectionWrite<PermManagerSnapshot, Bearer>
        + Send
        + Sync,
    Time: NetworkTimeProvider + Send + Sync,
    Actions: InflationActions<Bearer> + Send + Sync,
    Bearer: Send + Sync,
    Net: Network<Transaction, TxRejected> + Clone + std::marker::Sync + std::marker::Send,
{
    pub fn new(
        inflation_box: IB,
        poll_factory: PF,
        weighting_poll: WP,
        voting_escrow: VE,
        smart_farm: SF,
        perm_manager: PM,
        backlog: Backlog,
        ntp: Time,
        actions: Actions,
        conf: ProtocolConfig,
        pd: PhantomData<Bearer>,
        network: Net,
        prover: OperatorProver<'a>,
    ) -> Self {
        Self {
            inflation_box,
            poll_factory,
            weighting_poll,
            voting_escrow,
            smart_farm,
            perm_manager,
            backlog,
            ntp,
            actions,
            conf,
            pd,
            network,
            prover,
        }
    }
}

#[async_trait::async_trait]
impl<'a, IB, PF, WP, VE, SF, PM, Backlog, Time, Actions, Bearer, Net> RoutineBehaviour
    for Behaviour<'a, IB, PF, WP, VE, SF, PM, Backlog, Time, Actions, Bearer, Net>
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
    SF: StateProjectionRead<SmartFarmSnapshot, Bearer>
        + StateProjectionWrite<SmartFarmSnapshot, Bearer>
        + Send
        + Sync,
    PM: StateProjectionRead<PermManagerSnapshot, Bearer>
        + StateProjectionWrite<PermManagerSnapshot, Bearer>
        + Send
        + Sync,
    Time: NetworkTimeProvider + Send + Sync,
    Actions: InflationActions<Bearer> + Send + Sync,
    Bearer: Send + Sync,
    Net: Network<Transaction, TxRejected> + Clone + std::marker::Sync + std::marker::Send,
{
    async fn attempt(&mut self) -> Option<ToRoutine> {
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

impl<'a, IB, PF, WP, VE, SF, PM, Backlog, Time, Actions, Bearer, Net>
    Behaviour<'a, IB, PF, WP, VE, SF, PM, Backlog, Time, Actions, Bearer, Net>
{
    async fn inflation_box(&self) -> Option<AnyMod<Bundled<InflationBoxSnapshot, Bearer>>>
    where
        IB: StateProjectionRead<InflationBoxSnapshot, Bearer>,
    {
        self.inflation_box.read(InflationBoxId {}).await
    }

    async fn poll_factory(&self) -> Option<AnyMod<Bundled<PollFactorySnapshot, Bearer>>>
    where
        PF: StateProjectionRead<PollFactorySnapshot, Bearer>,
    {
        self.poll_factory
            .read(PollFactoryId(self.conf.deployed_validators.wp_factory.hash))
            .await
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

    async fn perm_manager(&self) -> Option<AnyMod<Bundled<PermManagerSnapshot, Bearer>>>
    where
        PM: StateProjectionRead<PermManagerSnapshot, Bearer>,
    {
        let token = (
            self.conf.tokens.perm_manager_auth_policy,
            AssetName::from(self.conf.tokens.perm_manager_auth_name.clone()),
        );
        self.perm_manager.read(PermManagerId(token)).await
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
        SF: StateProjectionRead<SmartFarmSnapshot, Bearer>,
        PM: StateProjectionRead<PermManagerSnapshot, Bearer>,
        VE: StateProjectionRead<VotingEscrowSnapshot, Bearer>,
        Backlog: ResilientBacklog<VotingOrder>,
        Time: NetworkTimeProvider,
    {
        if let (Some(inflation_box), Some(poll_factory), Some(perm_manager)) = (
            self.inflation_box().await,
            self.poll_factory().await,
            self.perm_manager().await,
        ) {
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
                            perm_manager,
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
        &mut self,
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
        Net: Network<Transaction, TxRejected> + Clone + std::marker::Sync,
    {
        if let (AnyMod::Confirmed(inflation_box), AnyMod::Confirmed(factory)) = (inflation_box, poll_factory)
        {
            let (signed_tx, next_inflation_box, next_factory, next_wpoll) = self
                .actions
                .create_wpoll(inflation_box.state.0, factory.state.0)
                .await;
            let tx = self.prover.prove(signed_tx);
            self.network.submit_tx(tx).await.unwrap();
            self.inflation_box.write_predicted(next_inflation_box).await;
            self.poll_factory.write_predicted(next_factory).await;
            self.weighting_poll.write_predicted(next_wpoll).await;
            return None;
        }
        retry_in(DEF_DELAY)
    }

    async fn try_apply_votes(
        &mut self,
        WeightingInProgress {
            weighting_poll,
            next_pending_order,
        }: WeightingInProgress<Bearer>,
    ) -> Option<ToRoutine>
    where
        WP: StateProjectionWrite<WeightingPollSnapshot, Bearer>,
        VE: StateProjectionWrite<VotingEscrowSnapshot, Bearer>,
        Actions: InflationActions<Bearer>,
        Net: Network<Transaction, TxRejected> + Clone + std::marker::Sync + std::marker::Send,
    {
        if let Some(next_order) = next_pending_order {
            let (signed_tx, next_wpoll, next_ve) = self
                .actions
                .execute_order(weighting_poll.erased(), next_order)
                .await;
            let tx = self.prover.prove(signed_tx);
            self.network.submit_tx(tx).await.unwrap();
            self.weighting_poll.write_predicted(next_wpoll).await;
            self.voting_escrow.write_predicted(next_ve).await;
            return None;
        }
        retry_in(DEF_DELAY)
    }

    async fn try_distribute_inflation(
        &mut self,
        DistributionInProgress {
            weighting_poll,
            next_farm,
            perm_manager,
            next_farm_weight,
        }: DistributionInProgress<Bearer>,
    ) where
        WP: StateProjectionWrite<WeightingPollSnapshot, Bearer>,
        SF: StateProjectionWrite<SmartFarmSnapshot, Bearer>,
        PM: StateProjectionWrite<PermManagerSnapshot, Bearer>,
        Actions: InflationActions<Bearer>,
        Net: Network<Transaction, TxRejected> + Clone + std::marker::Sync + std::marker::Send,
    {
        let (signed_tx, next_wpoll, next_sf, next_pm) = self
            .actions
            .distribute_inflation(
                weighting_poll.erased(),
                next_farm.erased(),
                perm_manager.erased(),
                next_farm_weight,
            )
            .await;
        let tx = self.prover.prove(signed_tx);
        self.network.submit_tx(tx).await.unwrap();
        self.weighting_poll.write_predicted(next_wpoll).await;
        self.smart_farm.write_predicted(next_sf).await;
        self.perm_manager.write_predicted(next_pm).await;
    }

    async fn try_eliminate_poll(
        &mut self,
        PendingEliminatePoll { weighting_poll }: PendingEliminatePoll<Bearer>,
    ) -> Option<ToRoutine>
    where
        Actions: InflationActions<Bearer>,
        Net: Network<Transaction, TxRejected> + Clone + std::marker::Sync + std::marker::Send,
    {
        if let AnyMod::Confirmed(Traced {
            state: Confirmed(weighting_poll),
            ..
        }) = weighting_poll
        {
            let signed_tx = self.actions.eliminate_wpoll(weighting_poll).await;
            let tx = self.prover.prove(signed_tx);
            self.network.submit_tx(tx).await.unwrap();
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
    next_farm: AnyMod<Bundled<SmartFarmSnapshot, Out>>,
    perm_manager: AnyMod<Bundled<PermManagerSnapshot, Out>>,
    next_farm_weight: u64,
}

pub struct PendingEliminatePoll<Out> {
    weighting_poll: AnyMod<Bundled<WeightingPollSnapshot, Out>>,
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use async_trait::async_trait;
    use serde::Serialize;
    use tokio::sync::Mutex;

    use bloom_offchain::execution_engine::bundled::Bundled;
    use spectrum_offchain::data::unique_entity::{AnyMod, Confirmed, Predicted, Traced};
    use spectrum_offchain::data::{EntitySnapshot, HasIdentifier, Identifier};

    use crate::state_projection::{StateProjectionRead, StateProjectionWrite};

    struct StateProjection<T: EntitySnapshot, B>(Arc<Mutex<Option<AnyMod<Bundled<T, B>>>>>);
    #[async_trait]
    impl<T, B> StateProjectionWrite<T, B> for StateProjection<T, B>
    where
        T: EntitySnapshot + HasIdentifier + Send + Sync + Clone,
        T::Version: Send,
        B: Send + Sync + Clone,
        T::Id: Send + Serialize + Sync,
    {
        async fn write_predicted(&self, entity: Traced<Predicted<Bundled<T, B>>>) {
            let _ = self.0.lock().await.insert(AnyMod::Predicted(entity));
        }

        async fn write_confirmed(&self, entity: Traced<Confirmed<Bundled<T, B>>>) {
            let _ = self.0.lock().await.insert(AnyMod::Confirmed(entity));
        }

        async fn remove(&self, id: T::Id) -> Option<T::Version> {
            // Stub
            None
        }
    }
    #[async_trait]
    impl<T, B> StateProjectionRead<T, B> for StateProjection<T, B>
    where
        T: EntitySnapshot + HasIdentifier + Send + Sync + Clone,
        T::Version: Send,
        B: Send + Sync + Clone,
        T::Id: Send + Serialize + Sync,
    {
        async fn read(&self, id: T::Id) -> Option<AnyMod<Bundled<T, B>>> {
            self.0.lock().await.clone()
        }
    }
}
