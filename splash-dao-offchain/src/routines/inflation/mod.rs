use std::collections::VecDeque;
use std::fmt::{Display, Formatter};
use std::hash::Hash;
use std::marker::PhantomData;
use std::pin::{pin, Pin};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use async_primitives::beacon::Beacon;
use async_stream::stream;
use bloom_offchain::execution_engine::bundled::Bundled;
use bloom_offchain::execution_engine::liquidity_book::core::Trans;
use bloom_offchain_cardano::event_sink::processed_tx::TxViewMut;
use cardano_chain_sync::data::LedgerTxEvent;
use cml_chain::plutus::{PlutusData, PlutusScript, PlutusV2Script};
use cml_chain::transaction::{Transaction, TransactionOutput};
use cml_chain::Serialize;
use cml_crypto::{PrivateKey, RawBytesEncoding, ScriptHash, TransactionHash};
use cml_multi_era::babbage::BabbageTransaction;
use either::Either;
use futures::{pin_mut, Future, FutureExt, Stream, StreamExt};
use futures_timer::Delay;
use isahc::http::header::RETRY_AFTER;
use log::{error, info, trace};
use pallas_network::miniprotocols::localtxsubmission::cardano_node_errors::{
    ApplyTxError, ConwayLedgerPredFailure, ConwayUtxoPredFailure, ConwayUtxowPredFailure,
};
use spectrum_cardano_lib::output::FinalizedTxOut;
use spectrum_cardano_lib::{AssetName, NetworkId, OutputRef};
use spectrum_offchain::backlog::data::{OrderWeight, Weighted};
use spectrum_offchain::backlog::ResilientBacklog;
use spectrum_offchain::data::circular_filter::CircularFilter;
use spectrum_offchain::domain::event::{AnyMod, Confirmed, Predicted, Traced, Unconfirmed};
use spectrum_offchain::domain::order::{PendingOrder, UniqueOrder};
use spectrum_offchain::domain::{EntitySnapshot, Has, Stable};
use spectrum_offchain::kv_store::KvStore;
use spectrum_offchain::ledger::TryFromLedger;
use spectrum_offchain::network::Network;
use spectrum_offchain::tx_prover::TxProver;
use spectrum_offchain_cardano::creds::operator_creds_base_address;
use spectrum_offchain_cardano::deployment::DeployedScriptInfo;
use spectrum_offchain_cardano::prover::operator::OperatorProver;
use spectrum_offchain_cardano::tx_submission::RejectReasons;
use tokio::runtime::Runtime;
use tokio::sync::mpsc::Receiver;
use tokio::sync::Mutex;
use type_equalities::IsEqual;

use crate::constants::time::{
    COOLDOWN_PERIOD_EXTRA_BUFFER, COOLDOWN_PERIOD_MILLIS, EPOCH_LEN, MAX_LOCK_TIME_SECONDS,
};
use crate::deployment::ProtocolValidator;
use crate::entities::offchain::voting_order::{VotingOrder, VotingOrderId};
use crate::entities::onchain::funding_box::{FundingBox, FundingBoxId, FundingBoxSnapshot};
use crate::entities::onchain::inflation_box::{InflationBoxId, InflationBoxSnapshot};
use crate::entities::onchain::make_voting_escrow_order::{
    MVEStatus, MakeVotingEscrowOrder, MakeVotingEscrowOrderBundle,
};
use crate::entities::onchain::permission_manager::{PermManager, PermManagerId, PermManagerSnapshot};
use crate::entities::onchain::poll_factory::{PollFactory, PollFactoryId, PollFactorySnapshot};
use crate::entities::onchain::smart_farm::{FarmId, SmartFarm, SmartFarmSnapshot};
use crate::entities::onchain::voting_escrow::{Owner, VotingEscrow, VotingEscrowId, VotingEscrowSnapshot};
use crate::entities::onchain::voting_escrow_factory::{VEFactoryId, VEFactorySnapshot};
use crate::entities::onchain::weighting_poll::{
    PollState, WeightingOngoing, WeightingPoll, WeightingPollId, WeightingPollSnapshot,
};
use crate::entities::onchain::{DaoEntity, DaoEntitySnapshot};
use crate::entities::Snapshot;
use crate::funding::FundingRepo;
use crate::protocol_config::{
    GTAuthPolicy, MintVECompositionPolicy, MintVEIdentifierPolicy, MintWPAuthPolicy,
    NotOutputRefNorSlotNumber, OperatorCreds, PermManagerAuthPolicy, ProtocolConfig, SplashPolicy,
    VEFactoryAuthPolicy,
};
use crate::routine::{retry_in, RoutineBehaviour, ToRoutine};
use crate::routines::inflation::actions::InflationActions;
use crate::state_projection::{StateProjectionRead, StateProjectionWrite};
use crate::time::{epoch_end, NetworkTimeProvider, ProtocolEpoch};
use crate::{CurrentEpoch, GenesisEpochStartTime, NetworkTimeSource};

pub mod actions;

pub struct Behaviour<
    IB,
    PF,
    VEF,
    WP,
    VE,
    SF,
    PM,
    FB,
    MVE,
    OVE,
    TMVE,
    OrderBacklog,
    PTX,
    Time,
    Actions,
    Bearer,
    Net,
> {
    inflation_box: IB,
    poll_factory: PF,
    ve_factory: VEF,
    weighting_poll: WP,
    voting_escrow: VE,
    smart_farm: SF,
    perm_manager: PM,
    funding_box: FB,
    /// Backlog of unspent `make_voting_escrow_order` UTxOs
    mve_order_backlog: MVE,
    /// Maps owners of `make_voting_escrow_order` to `MVEStatus` values. Used to respond to user
    /// queries regarding the state of their order.
    owner_to_voting_escrow: OVE,
    /// Maps an output reference to an associated `make_voting_escrow_order` UTxO. This is used to
    /// properly restore orders on chain-rollback.
    tx_hash_to_mve: TMVE,
    /// Backlog for all user voting orders.
    voting_order_backlog: OrderBacklog,
    predicted_tx_backlog: PTX,
    ntp: Time,
    actions: Actions,
    pub conf: ProtocolConfig,
    pd: PhantomData<Bearer>,
    network: Net,
    ledger_upstream: Receiver<LedgerTxEvent<TxViewMut>>,
    voting_orders: Receiver<DaoBotMessage>,
    chain_tip_reached: Arc<Mutex<bool>>,
    state_synced: Beacon,
    current_slot: Option<u64>,
    failed_to_confirm_txs_recv: Receiver<Transaction>,
}

const DEF_DELAY: Duration = Duration::new(5, 0);

#[async_trait::async_trait]
impl<IB, PF, VEF, WP, VE, SF, PM, FB, MVE, OVE, TMVE, OrderBacklog, PTX, Time, Actions, Bearer, Net>
    RoutineBehaviour
    for Behaviour<
        IB,
        PF,
        VEF,
        WP,
        VE,
        SF,
        PM,
        FB,
        MVE,
        OVE,
        TMVE,
        OrderBacklog,
        PTX,
        Time,
        Actions,
        Bearer,
        Net,
    >
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
    VEF: StateProjectionRead<VEFactorySnapshot, Bearer>
        + StateProjectionWrite<VEFactorySnapshot, Bearer>
        + Send
        + Sync,
    VE: StateProjectionRead<VotingEscrowSnapshot, Bearer>
        + StateProjectionWrite<VotingEscrowSnapshot, Bearer>
        + Send
        + Sync,
    OrderBacklog: ResilientBacklog<VotingOrder> + Send + Sync,
    PTX: KvStore<TransactionHash, PredictedEntityWrites<Bearer>> + Send + Sync,
    SF: StateProjectionRead<SmartFarmSnapshot, Bearer>
        + StateProjectionWrite<SmartFarmSnapshot, Bearer>
        + Send
        + Sync,
    PM: StateProjectionRead<PermManagerSnapshot, Bearer>
        + StateProjectionWrite<PermManagerSnapshot, Bearer>
        + Send
        + Sync,
    FB: FundingRepo + Send + Sync,
    MVE: ResilientBacklog<MakeVotingEscrowOrderBundle<Bearer>> + Send + Sync,
    OVE: KvStore<Owner, MVEStatus> + Send + Sync,
    TMVE: KvStore<TimedOutputRef, PendingOrder<MakeVotingEscrowOrderBundle<Bearer>>> + Send + Sync,
    Time: NetworkTimeProvider + Send + Sync,
    Actions: InflationActions<Bearer> + Send + Sync,
    Bearer: Send + Sync + std::fmt::Debug + Clone,
    Net: Network<Transaction, RejectReasons> + Clone + Sync + Send,
{
    async fn attempt(&mut self) -> Option<ToRoutine> {
        let RoutineState {
            previous_epoch_state,
            current_epoch_state,
            eliminate_wpoll,
        } = self.read_state().await;
        if let Some((wp_state, eliminated_epoch)) = eliminate_wpoll {
            trace!("Eliminating wpoll for epoch {}", eliminated_epoch);
            self.try_eliminate_poll(wp_state).await;
        }
        match previous_epoch_state {
            // For epoch 0
            None => match current_epoch_state {
                EpochRoutineState::Uninitialized
                | EpochRoutineState::WaitingForDistributionToStart
                | EpochRoutineState::WaitingToEliminate => retry_in(DEF_DELAY),
                EpochRoutineState::PendingCreatePoll(state) => self.try_create_wpoll(state).await,
                EpochRoutineState::WeightingInProgress(state) => {
                    trace!("Try making voting escrow (epoch 0)");
                    let _ = self.try_make_voting_escrow().await;
                    trace!("Try apply votes (epoch 0)");
                    self.try_apply_votes(state).await
                }
                EpochRoutineState::DistributionInProgress(state) => {
                    self.try_distribute_inflation(state).await
                }
                EpochRoutineState::PendingEliminatePoll(state) => self.try_eliminate_poll(state).await,
                EpochRoutineState::Eliminated => unreachable!(),
            },
            Some(EpochRoutineState::DistributionInProgress(prev_state)) => {
                trace!("Distributing inflation for previous epoch");
                self.try_distribute_inflation(prev_state).await
            }

            Some(EpochRoutineState::WaitingForDistributionToStart)
            | Some(EpochRoutineState::WaitingToEliminate)
            | Some(EpochRoutineState::Eliminated) => match current_epoch_state {
                EpochRoutineState::PendingCreatePoll(state) => {
                    trace!("Creating wpoll for current epoch");
                    self.try_create_wpoll(state).await
                }
                EpochRoutineState::WeightingInProgress(state) => {
                    trace!("Try apply votes for current epoch");
                    self.try_apply_votes(state).await
                }

                EpochRoutineState::WaitingToEliminate
                | EpochRoutineState::PendingEliminatePoll(_)
                | EpochRoutineState::WaitingForDistributionToStart => retry_in(DEF_DELAY),
                EpochRoutineState::Uninitialized => unreachable!(),
                EpochRoutineState::DistributionInProgress(_) => unreachable!(),
                EpochRoutineState::Eliminated => unreachable!(),
            },
            Some(EpochRoutineState::PendingEliminatePoll(_)) => retry_in(DEF_DELAY),
            Some(EpochRoutineState::WeightingInProgress(_)) => unreachable!(),
            Some(EpochRoutineState::PendingCreatePoll(_)) => unreachable!(),
            Some(EpochRoutineState::Uninitialized) => unreachable!(),
        }
    }
}

impl<IB, PF, VEF, WP, VE, SF, PM, FB, MVE, OVE, TMVE, Backlog, PTX, Time, Actions, Bearer, Net>
    Behaviour<IB, PF, VEF, WP, VE, SF, PM, FB, MVE, OVE, TMVE, Backlog, PTX, Time, Actions, Bearer, Net>
{
    pub fn new(
        inflation_box: IB,
        poll_factory: PF,
        weighting_poll: WP,
        ve_factory: VEF,
        voting_escrow: VE,
        smart_farm: SF,
        perm_manager: PM,
        funding_box: FB,
        make_voting_escrow_order: MVE,
        owner_to_voting_escrow: OVE,
        tx_hash_to_mve: TMVE,
        backlog: Backlog,
        predicted_tx_backlog: PTX,
        ntp: Time,
        actions: Actions,
        conf: ProtocolConfig,
        pd: PhantomData<Bearer>,
        network: Net,
        ledger_upstream: Receiver<LedgerTxEvent<TxViewMut>>,
        voting_orders: Receiver<DaoBotMessage>,
        state_synced: Beacon,
        failed_txs_recv: Receiver<Transaction>,
    ) -> Self
    where
        Backlog: ResilientBacklog<VotingOrder> + Send + Sync,
    {
        Self {
            inflation_box,
            poll_factory,
            weighting_poll,
            ve_factory,
            voting_escrow,
            smart_farm,
            perm_manager,
            funding_box,
            mve_order_backlog: make_voting_escrow_order,
            owner_to_voting_escrow,
            tx_hash_to_mve,
            voting_order_backlog: backlog,
            predicted_tx_backlog,
            ntp,
            actions,
            conf,
            pd,
            network,
            ledger_upstream,
            voting_orders,
            chain_tip_reached: Arc::new(Mutex::new(false)),
            state_synced,
            current_slot: None,
            failed_to_confirm_txs_recv: failed_txs_recv,
        }
    }

    async fn inflation_box(&self) -> Option<AnyMod<Bundled<InflationBoxSnapshot, Bearer>>>
    where
        IB: StateProjectionRead<InflationBoxSnapshot, Bearer> + Send + Sync,
        Time: NetworkTimeProvider + Send + Sync,
    {
        let time_millis = self.ntp.network_time().await * 1000;
        let current_epoch = time_millis_to_epoch(time_millis, self.conf.genesis_time).0;
        for epoch in (0..=current_epoch).rev() {
            if let Some(ib) = self.inflation_box.read(InflationBoxId(epoch)).await {
                return Some(ib);
            }
        }
        None
    }

    async fn poll_factory(&self) -> Option<AnyMod<Bundled<PollFactorySnapshot, Bearer>>>
    where
        PF: StateProjectionRead<PollFactorySnapshot, Bearer> + Send + Sync,
        Time: NetworkTimeProvider + Send + Sync,
    {
        let time_millis = self.ntp.network_time().await * 1000;
        let current_epoch = time_millis_to_epoch(time_millis, self.conf.genesis_time).0;
        for epoch in (0..=current_epoch).rev() {
            if let Some(pf) = self.poll_factory.read(PollFactoryId(epoch)).await {
                return Some(pf);
            }
        }
        None
    }

    async fn weighting_poll(
        &self,
        epoch: ProtocolEpoch,
    ) -> Option<Either<WPollEliminated, AnyMod<Bundled<WeightingPollSnapshot, Bearer>>>>
    where
        WP: StateProjectionRead<WeightingPollSnapshot, Bearer> + Send + Sync,
    {
        self.weighting_poll.read(self.conf.poll_id(epoch)).await.map(|a| {
            let ver = a.as_erased().version();
            let wpoll = a.as_erased().0.get();
            if wpoll.eliminated {
                // We don't return a weighting_poll that's already been eliminated
                trace!(
                    "Behaviour::weighting_poll({}) with ver: {} already eliminated -------------",
                    epoch,
                    ver
                );
                Either::Left(WPollEliminated)
            } else {
                Either::Right(a)
            }
        })
    }

    async fn perm_manager(&self) -> Option<AnyMod<Bundled<PermManagerSnapshot, Bearer>>>
    where
        PM: StateProjectionRead<PermManagerSnapshot, Bearer> + Send + Sync,
    {
        self.perm_manager.read(PermManagerId {}).await
    }

    async fn next_order(
        &self,
        _stage: WeightingOngoing,
    ) -> Option<(VotingOrder, Bundled<VotingEscrowSnapshot, Bearer>)>
    where
        VE: StateProjectionRead<VotingEscrowSnapshot, Bearer> + Send + Sync,
        Backlog: ResilientBacklog<VotingOrder> + Send + Sync,
        Bearer: std::fmt::Debug,
    {
        if let Some(ord) = self.voting_order_backlog.try_pop().await {
            let ve_id = ord.id.voting_escrow_id.0;
            info!("Executing order with VE_identifier {}", ve_id);
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
        IB: StateProjectionRead<InflationBoxSnapshot, Bearer> + Send + Sync,
        PF: StateProjectionRead<PollFactorySnapshot, Bearer> + Send + Sync,
        PM: StateProjectionRead<PermManagerSnapshot, Bearer> + Send + Sync,
        WP: StateProjectionRead<WeightingPollSnapshot, Bearer> + Send + Sync,
        VE: StateProjectionRead<VotingEscrowSnapshot, Bearer> + Send + Sync,
        SF: StateProjectionRead<SmartFarmSnapshot, Bearer> + Send + Sync,
        VEF: StateProjectionRead<VEFactorySnapshot, Bearer> + Send + Sync,
        Backlog: ResilientBacklog<VotingOrder> + Send + Sync,
        Bearer: std::fmt::Debug + Clone,
        Time: NetworkTimeProvider + Send + Sync,
    {
        let ibox = self.inflation_box().await;
        let wp_factory = self.poll_factory().await;
        let perm_manager = self.perm_manager().await;
        if let (Some(inflation_box), Some(poll_factory), Some(perm_manager)) =
            (ibox, wp_factory, perm_manager)
        {
            let genesis = self.conf.genesis_time;
            let now_millis = self.ntp.network_time().await * 1000;
            let current_epoch = inflation_box
                .as_erased()
                .0
                .get()
                .active_epoch(genesis, now_millis);
            let (previous_epoch_state, eliminate_wpoll) = if current_epoch > 0 {
                let eliminate_wpoll = self
                    .get_latest_wpoll_to_eliminate(current_epoch - 1, genesis, now_millis)
                    .await
                    .map(|(weighting_poll, epoch)| (PendingEliminatePoll { weighting_poll }, epoch));
                let previous_epoch_state = match self.weighting_poll(current_epoch - 1).await {
                    Some(Either::Right(prev_wp)) => {
                        match prev_wp.as_erased().0.get().state(genesis, now_millis) {
                            PollState::WeightingOngoing(_st) => {
                                unreachable!("Weighting is over for epoch {}", current_epoch - 1);
                            }
                            PollState::DistributionOngoing(next_farm) => {
                                trace!("Previous wpoll still exists, distributing inflation");
                                Some(EpochRoutineState::DistributionInProgress(
                                    DistributionInProgress {
                                        next_farm: self
                                            .smart_farm
                                            .read(next_farm.farm_id())
                                            .await
                                            .expect("State is inconsistent"),
                                        weighting_poll: prev_wp,
                                        next_farm_weight: next_farm.farm_weight(),
                                        perm_manager: perm_manager.clone(),
                                    },
                                ))
                            }
                            PollState::PollExhaustedButNotReadyToEliminate => {
                                Some(EpochRoutineState::WaitingToEliminate)
                            }
                            PollState::PollExhaustedAndReadyToEliminate => {
                                Some(EpochRoutineState::PendingEliminatePoll(PendingEliminatePoll {
                                    weighting_poll: prev_wp,
                                }))
                            }
                            PollState::Eliminated => Some(EpochRoutineState::Eliminated),
                            PollState::WaitingForDistributionToStart => {
                                trace!("Waiting for distribution of epoch {} to start", current_epoch - 1);
                                Some(EpochRoutineState::WaitingForDistributionToStart)
                            }
                        }
                    }
                    Some(Either::Left(WPollEliminated)) => Some(EpochRoutineState::Eliminated),
                    None => None,
                };
                (previous_epoch_state, eliminate_wpoll)
            } else {
                (None, None)
            };
            let current_epoch_state = match self.weighting_poll(current_epoch).await {
                None => {
                    trace!("No weighting_poll @ epoch {}, creating it...", current_epoch);
                    EpochRoutineState::PendingCreatePoll(PendingCreatePoll {
                        inflation_box,
                        poll_factory,
                    })
                }
                Some(Either::Right(wp)) => match wp.as_erased().0.get().state(genesis, now_millis) {
                    PollState::WeightingOngoing(st) => {
                        trace!("Weighting on going @ epoch {}", current_epoch);

                        EpochRoutineState::WeightingInProgress(WeightingInProgress {
                            weighting_poll: wp,
                            next_pending_order: self.next_order(st).await,
                        })
                    }
                    PollState::DistributionOngoing(_) => {
                        unreachable!("Impossible to distribute inflation on current epoch");
                    }
                    PollState::PollExhaustedAndReadyToEliminate => {
                        unreachable!("Impossible to eliminate wpoll on current epoch");
                    }
                    PollState::PollExhaustedButNotReadyToEliminate => {
                        trace!("WPoll in current epoch exhausted");
                        EpochRoutineState::WaitingToEliminate
                    }
                    PollState::Eliminated => EpochRoutineState::Eliminated,
                    PollState::WaitingForDistributionToStart => {
                        EpochRoutineState::WaitingForDistributionToStart
                    }
                },
                Some(Either::Left(WPollEliminated)) => unreachable!(),
            };

            RoutineState {
                previous_epoch_state,
                current_epoch_state,
                eliminate_wpoll,
            }
        } else {
            RoutineState {
                previous_epoch_state: None,
                current_epoch_state: EpochRoutineState::Uninitialized,
                eliminate_wpoll: None,
            }
        }
    }

    pub async fn confirm_entity(&mut self, Bundled(entity, bearer): Bundled<DaoEntitySnapshot, Bearer>)
    where
        IB: StateProjectionRead<InflationBoxSnapshot, Bearer>
            + StateProjectionWrite<InflationBoxSnapshot, Bearer>
            + Send
            + Sync,
        PF: StateProjectionRead<PollFactorySnapshot, Bearer>
            + StateProjectionWrite<PollFactorySnapshot, Bearer>
            + Send
            + Sync,
        VEF: StateProjectionRead<VEFactorySnapshot, Bearer>
            + StateProjectionWrite<VEFactorySnapshot, Bearer>
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
        OVE: KvStore<Owner, MVEStatus> + Send + Sync,
        MVE: ResilientBacklog<MakeVotingEscrowOrderBundle<Bearer>> + Send + Sync,
        TMVE: KvStore<TimedOutputRef, PendingOrder<MakeVotingEscrowOrderBundle<Bearer>>> + Send + Sync,
        Backlog: ResilientBacklog<VotingOrder> + Send + Sync,
        SF: StateProjectionRead<SmartFarmSnapshot, Bearer>
            + StateProjectionWrite<SmartFarmSnapshot, Bearer>
            + Send
            + Sync,
        PM: StateProjectionRead<PermManagerSnapshot, Bearer>
            + StateProjectionWrite<PermManagerSnapshot, Bearer>
            + Send
            + Sync,
        FB: FundingRepo + Send + Sync,
        Time: NetworkTimeProvider + Send + Sync,
        Bearer: Clone,
    {
        match entity.get() {
            DaoEntity::Inflation(ib) => {
                let confirmed_snapshot = Confirmed(Bundled(Snapshot::new(*ib, *entity.version()), bearer));
                let traced = Traced {
                    state: confirmed_snapshot,
                    prev_state_id: None,
                };
                self.inflation_box.write_confirmed(traced).await;
            }
            DaoEntity::PermManager(pm) => {
                let confirmed_snapshot =
                    Confirmed(Bundled(Snapshot::new(pm.clone(), *entity.version()), bearer));
                let prev_state_id = if let Some(state) = self.perm_manager.read(PermManagerId).await {
                    let bundled = state.erased();
                    Some(bundled.version())
                } else {
                    None
                };
                let traced = Traced {
                    state: confirmed_snapshot,
                    prev_state_id,
                };
                self.perm_manager.write_confirmed(traced).await;
            }
            DaoEntity::WeightingPollFactory(wp_factory) => {
                let confirmed_snapshot = Confirmed(Bundled(
                    Snapshot::new(wp_factory.clone(), *entity.version()),
                    bearer,
                ));

                let traced = Traced {
                    state: confirmed_snapshot,
                    prev_state_id: None,
                };
                self.poll_factory.write_confirmed(traced).await;
            }
            DaoEntity::SmartFarm(sf) => {
                let confirmed_snapshot =
                    Confirmed(Bundled(Snapshot::new(sf.clone(), *entity.version()), bearer));
                let prev_state_id = if let Some(state) = self.smart_farm.read(sf.farm_id).await {
                    let bundled = state.erased();
                    Some(bundled.version())
                } else {
                    None
                };
                let traced = Traced {
                    state: confirmed_snapshot,
                    prev_state_id,
                };
                self.smart_farm.write_confirmed(traced).await;
            }
            DaoEntity::VotingEscrow(ve) => {
                let confirmed_snapshot =
                    Confirmed(Bundled(Snapshot::new(ve.clone(), *entity.version()), bearer));
                let prev_state_id = if let Some(state) = self
                    .voting_escrow
                    .read(VotingEscrowId::from(ve.ve_identifier_name))
                    .await
                {
                    let bundled = state.erased();
                    Some(bundled.version())
                } else {
                    None
                };
                let traced = Traced {
                    state: confirmed_snapshot,
                    prev_state_id,
                };
                self.voting_escrow.write_confirmed(traced).await;
            }
            DaoEntity::VotingEscrowFactory(ve_factory) => {
                let confirmed_snapshot = Confirmed(Bundled(
                    Snapshot::new(ve_factory.clone(), *entity.version()),
                    bearer,
                ));
                let prev_state_id = if let Some(state) = self.ve_factory.read(VEFactoryId).await {
                    let bundled = state.erased();
                    Some(bundled.version())
                } else {
                    None
                };
                let traced = Traced {
                    state: confirmed_snapshot,
                    prev_state_id,
                };
                self.ve_factory.write_confirmed(traced).await;
            }
            DaoEntity::WeightingPoll(wp) => {
                let confirmed_snapshot =
                    Confirmed(Bundled(Snapshot::new(wp.clone(), *entity.version()), bearer));
                let prev_state_id =
                    if let Some(state) = self.weighting_poll.read(WeightingPollId::from(wp.epoch)).await {
                        let bundled = state.erased();
                        Some(bundled.version())
                    } else {
                        None
                    };
                let traced = Traced {
                    state: confirmed_snapshot,
                    prev_state_id,
                };
                trace!(
                    "Weighting_poll confirmed: epoch {}, version: {:?}, prev_version: {:?}",
                    wp.epoch,
                    entity.version(),
                    prev_state_id,
                );
                self.weighting_poll.write_confirmed(traced).await;
            }

            DaoEntity::FundingBox(fb) => {
                self.funding_box.put_confirmed(Confirmed(fb.clone())).await;
            }
            DaoEntity::MakeVotingEscrowOrder(make_voting_escrow_order) => {
                trace!(
                    "make_voting_escrow_order confirmed: owner {}, version: {:?}",
                    make_voting_escrow_order.ve_datum.owner,
                    entity.version(),
                );
                let time_src = NetworkTimeSource {};
                let timestamp = time_src.network_time().await as i64;
                let owner = make_voting_escrow_order.ve_datum.owner;
                let order = MakeVotingEscrowOrderBundle::new(
                    make_voting_escrow_order.clone(),
                    *entity.version(),
                    bearer,
                );
                let ord = PendingOrder { order, timestamp };
                self.mve_order_backlog.put(ord.clone()).await;
                self.tx_hash_to_mve.insert(*entity.version(), ord).await;
                self.owner_to_voting_escrow
                    .insert(owner, MVEStatus::Unspent)
                    .await;
            }
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
        Actions: InflationActions<Bearer> + Send + Sync,
        Net: Network<Transaction, RejectReasons> + Clone + Sync + Send,
        IB: StateProjectionWrite<InflationBoxSnapshot, Bearer> + Send + Sync,
        PF: StateProjectionWrite<PollFactorySnapshot, Bearer> + Send + Sync,
        WP: StateProjectionWrite<WeightingPollSnapshot, Bearer> + Send + Sync,
        PTX: KvStore<TransactionHash, PredictedEntityWrites<Bearer>> + Send + Sync,
        FB: FundingRepo + Send + Sync,
        Time: NetworkTimeProvider + Send + Sync,
    {
        if let (AnyMod::Confirmed(inflation_box), AnyMod::Confirmed(factory)) = (inflation_box, poll_factory)
        {
            if self.current_slot.is_none() {
                return retry_in(DEF_DELAY);
            }
            let current_slot = self.current_slot.unwrap();
            let funding_boxes = AvailableFundingBoxes(self.funding_box.collect().await.unwrap());
            let lovelaces_input_value = funding_boxes.0.iter().fold(0, |acc, x| acc + x.value.coin);
            if lovelaces_input_value >= 5_000_000 {
                let (signed_tx, next_inflation_box, next_factory, next_wpoll, funding_box_changes) = self
                    .actions
                    .create_wpoll(
                        inflation_box.state.0,
                        factory.state.0,
                        Slot(current_slot),
                        funding_boxes,
                    )
                    .await;
                let prover = OperatorProver::new(self.conf.operator_sk.clone());
                let outbound_tx = prover.prove(signed_tx);
                let tx = outbound_tx.clone();
                let tx_hash = tx.body.hash();
                info!("`create_wpoll`: submitting TX (hash: {})", tx_hash);
                match self.network.submit_tx(outbound_tx).await {
                    Ok(()) => {
                        let inflation_box_id = next_inflation_box.state.stable_id();
                        let wp_factory_id = next_factory.state.stable_id();
                        let wpoll_id = next_wpoll.state.stable_id();
                        let predicted_write = PredictedEntityWrites::CreateWPoll {
                            inflation_box_id,
                            wp_factory_id,
                            wpoll_id,
                            funding_box_changes: funding_box_changes.clone(),
                            tx_hash,
                        };
                        self.predicted_tx_backlog.insert(tx_hash, predicted_write).await;
                        info!("`create_wpoll`: TX submission SUCCESS");
                        self.inflation_box.write_predicted(next_inflation_box).await;
                        self.poll_factory.write_predicted(next_factory).await;
                        self.weighting_poll.write_predicted(next_wpoll).await;

                        for p in funding_box_changes.spent {
                            self.funding_box.spend_predicted(p).await;
                        }

                        for fb in funding_box_changes.created {
                            self.funding_box.put_predicted(fb).await;
                        }

                        return None;
                    }
                    Err(RejectReasons(Some(ApplyTxError { node_errors }))) => {
                        if node_errors.iter().any(|err| {
                            matches!(
                                err,
                                ConwayLedgerPredFailure::UtxowFailure(ConwayUtxowPredFailure::UtxoFailure(
                                    ConwayUtxoPredFailure::BadInputsUtxo(_)
                                ),)
                            )
                        }) {
                            info!("`create_wpoll`: Bad/missing input UTxO. Retrying...");
                            return None;
                        } else {
                            // For all other errors we discard the order.
                            error!("`create_wpoll`: TX submit failed on errors: {:?}", node_errors);
                            return None;
                        }
                    }
                    Err(RejectReasons(None)) => {
                        error!("`create_wpoll`: TX submit failed on UNKNOWN error");
                        return None;
                    }
                }
            } else {
                info!("`create_wpoll`: Insufficient ADA. Waiting for other funding boxes to be confirmed.");
            }
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
        Actions: InflationActions<Bearer> + Send + Sync,
        Net: Network<Transaction, RejectReasons> + Clone + Sync + Send,
        WP: StateProjectionWrite<WeightingPollSnapshot, Bearer> + Send + Sync,
        VE: StateProjectionWrite<VotingEscrowSnapshot, Bearer> + Send + Sync,
        Backlog: ResilientBacklog<VotingOrder> + Send + Sync,
        FB: FundingRepo + Send + Sync,
        PTX: KvStore<TransactionHash, PredictedEntityWrites<Bearer>> + Send + Sync,
    {
        if let Some(next_order) = next_pending_order {
            if self.current_slot.is_none() {
                return retry_in(DEF_DELAY);
            }
            let current_slot = self.current_slot.unwrap();
            let order = next_order.0.clone();
            let order_id = order.id;
            match self
                .actions
                .execute_order(weighting_poll.erased(), next_order, Slot(current_slot))
                .await
            {
                Ok((signed_tx, next_wpoll, next_ve)) => {
                    let prover = OperatorProver::new(self.conf.operator_sk.clone());
                    let outbound_tx = prover.prove(signed_tx);
                    let tx = outbound_tx.clone();
                    let tx_hash = tx.body.hash();
                    info!("`execute_order`: submitting TX (hash: {})", tx_hash);
                    match self.network.submit_tx(outbound_tx).await {
                        Ok(()) => {
                            info!("`execute_order`: TX submission SUCCESS");

                            let wpoll_id = next_wpoll.state.stable_id();
                            let voting_escrow_id = next_ve.state.stable_id();
                            let predicted_write = PredictedEntityWrites::ApplyVotingOrder {
                                wpoll_id,
                                voting_escrow_id,
                                voting_order: order,
                                tx_hash,
                            };
                            self.predicted_tx_backlog.insert(tx_hash, predicted_write).await;

                            self.weighting_poll.write_predicted(next_wpoll).await;
                            self.voting_escrow.write_predicted(next_ve).await;
                            self.voting_order_backlog.remove(order_id).await;

                            return None;
                        }
                        Err(RejectReasons(Some(ApplyTxError { node_errors }))) => {
                            // We suspend the order if there are bad/missing inputs. With this TX the
                            // only inputs are `weighting_poll` and `voting_escrow`. If they're missing
                            // from the UTxO set then it's possible that another bot has made a TX
                            // involving at least one of these inputs.
                            if node_errors.iter().any(|err| {
                                matches!(
                                    err,
                                    ConwayLedgerPredFailure::UtxowFailure(
                                        ConwayUtxowPredFailure::UtxoFailure(
                                            ConwayUtxoPredFailure::BadInputsUtxo(_)
                                        ),
                                    )
                                )
                            }) {
                                info!("`execute_order`: TX failed on bad/missing input error");
                                self.voting_order_backlog.suspend(order).await;
                                return None;
                            } else {
                                // For all other errors we discard the order.
                                error!("`execute_order`: TX submit failed on errors: {:?}", node_errors);
                                self.voting_order_backlog.remove(order_id).await;
                                return None;
                            }
                        }
                        Err(RejectReasons(None)) => {
                            error!("`execute_order`: TX submit failed on unknown error");
                            self.voting_order_backlog.remove(order_id).await;
                            return None;
                        }
                    }
                }
                Err(e) => {
                    error!("`execute_order`: Inadmissible order, error: {:?}", e);
                    // Here the order has been deemed inadmissible and so it will be removed.
                    self.voting_order_backlog.remove(order_id).await;
                    return None;
                }
            }
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
    ) -> Option<ToRoutine>
    where
        Actions: InflationActions<Bearer> + Send + Sync,
        Net: Network<Transaction, RejectReasons> + Clone + Sync + Send,
        WP: StateProjectionWrite<WeightingPollSnapshot, Bearer> + Send + Sync,
        SF: StateProjectionWrite<SmartFarmSnapshot, Bearer> + Send + Sync,
        PM: StateProjectionWrite<PermManagerSnapshot, Bearer> + Send + Sync,
        FB: FundingRepo + Send + Sync,
        PTX: KvStore<TransactionHash, PredictedEntityWrites<Bearer>> + Send + Sync,
    {
        if self.current_slot.is_none() {
            return retry_in(DEF_DELAY);
        }
        let current_slot = self.current_slot.unwrap();
        let funding_boxes = AvailableFundingBoxes(self.funding_box.collect().await.unwrap());
        let lovelaces_input_value = funding_boxes.0.iter().fold(0, |acc, x| acc + x.value.coin);
        if lovelaces_input_value >= 5_000_000 {
            let (signed_tx, next_wpoll, next_sf, funding_box_changes) = self
                .actions
                .distribute_inflation(
                    weighting_poll.erased(),
                    next_farm.erased(),
                    perm_manager.erased(),
                    Slot(current_slot),
                    next_farm_weight,
                    funding_boxes,
                )
                .await;
            let prover = OperatorProver::new(self.conf.operator_sk.clone());
            let outbound_tx = prover.prove(signed_tx);
            let tx = outbound_tx.clone();
            let tx_hash = tx.body.hash();
            info!("`distribute_inflation`: submitting TX (hash: {})", tx_hash);
            match self.network.submit_tx(outbound_tx).await {
                Ok(()) => {
                    info!("`distribute_inflation`: TX submission SUCCESS");

                    let predicted_write = PredictedEntityWrites::DistributeInflation {
                        wpoll_id: next_wpoll.state.stable_id(),
                        smart_farm_id: next_sf.state.stable_id(),
                        funding_box_changes: funding_box_changes.clone(),
                        tx_hash,
                    };
                    self.predicted_tx_backlog.insert(tx_hash, predicted_write).await;
                    self.weighting_poll.write_predicted(next_wpoll).await;
                    self.smart_farm.write_predicted(next_sf).await;
                    for p in funding_box_changes.spent {
                        self.funding_box.spend_predicted(p).await;
                    }

                    for fb in funding_box_changes.created {
                        self.funding_box.put_predicted(fb).await;
                    }
                }
                Err(RejectReasons(Some(ApplyTxError { node_errors }))) => {
                    if node_errors.iter().any(|err| {
                        matches!(
                            err,
                            ConwayLedgerPredFailure::UtxowFailure(ConwayUtxowPredFailure::UtxoFailure(
                                ConwayUtxoPredFailure::BadInputsUtxo(_)
                            ),)
                        )
                    }) {
                        info!("`distribute_inflation`: Bad/missing input UTxO. Retrying...");
                        return None;
                    } else {
                        // For all other errors we discard the order.
                        error!(
                            "`distribute_inflation`: TX submit failed on errors: {:?}",
                            node_errors
                        );
                        return None;
                    }
                }
                Err(RejectReasons(None)) => {
                    error!("`distribute_inflation`: TX submit failed on UNKNOWN error");
                    return None;
                }
            }
        } else {
            info!(
                "`distribute_inflation`: Insufficient ADA. Waiting for other funding boxes to be confirmed."
            );
        }
        retry_in(DEF_DELAY)
    }

    async fn try_eliminate_poll(
        &mut self,
        PendingEliminatePoll { weighting_poll }: PendingEliminatePoll<Bearer>,
    ) -> Option<ToRoutine>
    where
        Actions: InflationActions<Bearer> + Send + Sync,
        Net: Network<Transaction, RejectReasons> + Clone + Sync + Send,
        FB: FundingRepo + Send + Sync,
        PTX: KvStore<TransactionHash, PredictedEntityWrites<Bearer>> + Send + Sync,
    {
        if let AnyMod::Confirmed(Traced {
            state: Confirmed(weighting_poll),
            ..
        }) = weighting_poll
        {
            if self.current_slot.is_none() {
                return retry_in(DEF_DELAY);
            }
            let current_slot = self.current_slot.unwrap();
            let wp = weighting_poll.0.get();
            let epoch = wp.epoch;
            let time_millis = slot_to_time_millis(current_slot, NetworkId::from(0));

            let funding_boxes = AvailableFundingBoxes(self.funding_box.collect().await.unwrap());
            let lovelaces_input_value = funding_boxes.0.iter().fold(0, |acc, x| acc + x.value.coin);
            if lovelaces_input_value >= 3_000_000 && wp.can_be_eliminated(self.conf.genesis_time, time_millis)
            {
                info!("Eliminating wpoll @ epoch {}", epoch);
                let (signed_tx, funding_box_changes) = self
                    .actions
                    .eliminate_wpoll(weighting_poll, funding_boxes, Slot(current_slot))
                    .await;
                let prover = OperatorProver::new(self.conf.operator_sk.clone());
                let outbound_tx = prover.prove(signed_tx);
                let tx = outbound_tx.clone();
                let tx_hash = tx.body.hash();
                match self.network.submit_tx(outbound_tx).await {
                    Ok(()) => {
                        let predicted_write = PredictedEntityWrites::EiminateWPoll {
                            funding_box_changes: funding_box_changes.clone(),
                            tx_hash,
                        };
                        self.predicted_tx_backlog.insert(tx_hash, predicted_write).await;
                        info!(
                            "Eliminating wpoll @ epoch {} SUCCESS (tx hash: {})",
                            epoch, tx_hash
                        );

                        for p in funding_box_changes.spent {
                            self.funding_box.spend_predicted(p).await;
                        }

                        for fb in funding_box_changes.created {
                            self.funding_box.put_predicted(fb).await;
                        }
                        return None;
                    }
                    Err(RejectReasons(Some(ApplyTxError { node_errors }))) => {
                        if node_errors.iter().any(|err| {
                            matches!(
                                err,
                                ConwayLedgerPredFailure::UtxowFailure(ConwayUtxowPredFailure::UtxoFailure(
                                    ConwayUtxoPredFailure::BadInputsUtxo(_)
                                ),)
                            )
                        }) {
                            info!("`eliminate_wpoll`: Bad/missing input UTxO. Retrying...");
                            return None;
                        } else {
                            // For all other errors we discard the order.
                            error!("`eliminate_wpoll`: TX submit failed on errors: {:?}", node_errors);
                            return None;
                        }
                    }
                    Err(RejectReasons(None)) => {
                        error!("`eliminate_wpoll`: TX submit failed on UNKNOWN error");
                        return None;
                    }
                }
            }
        }
        retry_in(DEF_DELAY)
    }

    async fn try_make_voting_escrow(&mut self) -> Option<ToRoutine>
    where
        Actions: InflationActions<Bearer> + Send + Sync,
        Net: Network<Transaction, RejectReasons> + Clone + Sync + Send,
        MVE: ResilientBacklog<MakeVotingEscrowOrderBundle<Bearer>> + Send + Sync,
        OVE: KvStore<Owner, MVEStatus> + Send + Sync,
        VE: StateProjectionRead<VotingEscrowSnapshot, Bearer>
            + StateProjectionWrite<VotingEscrowSnapshot, Bearer>
            + Send
            + Sync,
        VEF: StateProjectionRead<VEFactorySnapshot, Bearer>
            + StateProjectionWrite<VEFactorySnapshot, Bearer>
            + Send
            + Sync,
        PTX: KvStore<TransactionHash, PredictedEntityWrites<Bearer>> + Send + Sync,
        Bearer: Clone,
    {
        if self.current_slot.is_none() {
            return retry_in(DEF_DELAY);
        }
        let current_slot = Slot(self.current_slot.unwrap());

        if let Some(mve_order) = self.mve_order_backlog.try_pop().await {
            let ve_factory = self.ve_factory.read(VEFactoryId).await.unwrap().erased();
            let result = self
                .actions
                .make_voting_escrow(mve_order.clone(), ve_factory, current_slot)
                .await;
            match result {
                Ok((signed_tx, next_ve_factory, next_ve)) => {
                    let prover = OperatorProver::new(self.conf.operator_sk.clone());
                    let outbound_tx = prover.prove(signed_tx);
                    let tx = outbound_tx.clone();
                    let tx_hash = tx.body.hash();
                    info!("`make_voting_escrow`: submitting TX (hash: {})", tx_hash);
                    match self.network.submit_tx(outbound_tx).await {
                        Ok(()) => {
                            let voting_escrow_id = next_ve.state.stable_id();
                            let predicted_write = PredictedEntityWrites::MakeVotingEscrow {
                                tx_hash,
                                voting_escrow_id,
                                mve_order,
                            };
                            self.predicted_tx_backlog.insert(tx_hash, predicted_write).await;
                            info!(
                                "Created voting_escrow with id = {}: SUCCESS (tx hash: {})",
                                voting_escrow_id, tx_hash
                            );
                            self.ve_factory.write_predicted(next_ve_factory).await;
                            self.voting_escrow.write_predicted(next_ve).await;

                            return None;
                        }
                        Err(RejectReasons(Some(ApplyTxError { node_errors }))) => {
                            if node_errors.iter().any(|err| {
                                matches!(
                                    err,
                                    ConwayLedgerPredFailure::UtxowFailure(
                                        ConwayUtxowPredFailure::UtxoFailure(
                                            ConwayUtxoPredFailure::BadInputsUtxo(_)
                                        ),
                                    )
                                )
                            }) {
                                info!("`make_voting_escrow`: Bad/missing input UTxO. Retrying...");
                                self.mve_order_backlog.suspend(mve_order).await;
                                return None;
                            } else {
                                // For all other errors we discard the order.
                                error!(
                                    "`make_voting_escrow`: TX submit failed on errors: {:?}",
                                    node_errors
                                );
                                self.mve_order_backlog
                                    .remove(mve_order.output_ref.output_ref)
                                    .await;
                                return None;
                            }
                        }
                        Err(RejectReasons(None)) => {
                            error!("`make_voting_escrow`: TX submit failed on UNKNOWN error");
                            self.mve_order_backlog
                                .remove(mve_order.output_ref.output_ref)
                                .await;
                            return None;
                        }
                    }
                }
                Err(e) => {
                    error!("`make_voting_escrow`: invalid order, error: {:?}", e);
                    self.mve_order_backlog
                        .remove(mve_order.output_ref.output_ref)
                        .await;
                    return None;
                }
            }
        }
        retry_in(DEF_DELAY)
    }

    async fn get_latest_wpoll_to_eliminate(
        &self,
        starting_epoch: ProtocolEpoch,
        genesis: GenesisEpochStartTime,
        now_millis: u64,
    ) -> Option<(AnyMod<Bundled<WeightingPollSnapshot, Bearer>>, ProtocolEpoch)>
    where
        WP: StateProjectionRead<WeightingPollSnapshot, Bearer> + Send + Sync,
    {
        for epoch in (1..=starting_epoch).rev() {
            if let Some(Either::Right(wp)) = self.weighting_poll(epoch).await {
                if let PollState::PollExhaustedAndReadyToEliminate =
                    wp.as_erased().0.get().state(genesis, now_millis)
                {
                    return Some((wp, epoch));
                }
            }
        }
        None
    }
}

impl<IB, PF, VEF, WP, VE, SF, PM, FB, MVE, OVE, TMVE, Backlog, PTX, Time, Actions, Net>
    Behaviour<
        IB,
        PF,
        VEF,
        WP,
        VE,
        SF,
        PM,
        FB,
        MVE,
        OVE,
        TMVE,
        Backlog,
        PTX,
        Time,
        Actions,
        TransactionOutput,
        Net,
    >
where
    IB: StateProjectionRead<InflationBoxSnapshot, TransactionOutput>
        + StateProjectionWrite<InflationBoxSnapshot, TransactionOutput>
        + Send
        + Sync,
    PF: StateProjectionRead<PollFactorySnapshot, TransactionOutput>
        + StateProjectionWrite<PollFactorySnapshot, TransactionOutput>
        + Send
        + Sync,
    VEF: StateProjectionRead<VEFactorySnapshot, TransactionOutput>
        + StateProjectionWrite<VEFactorySnapshot, TransactionOutput>
        + Send
        + Sync,
    WP: StateProjectionRead<WeightingPollSnapshot, TransactionOutput>
        + StateProjectionWrite<WeightingPollSnapshot, TransactionOutput>
        + Send
        + Sync,
    VE: StateProjectionRead<VotingEscrowSnapshot, TransactionOutput>
        + StateProjectionWrite<VotingEscrowSnapshot, TransactionOutput>
        + Send
        + Sync,
    Backlog: ResilientBacklog<VotingOrder> + Send + Sync,
    PTX: KvStore<TransactionHash, PredictedEntityWrites<TransactionOutput>> + Send + Sync,
    SF: StateProjectionRead<SmartFarmSnapshot, TransactionOutput>
        + StateProjectionWrite<SmartFarmSnapshot, TransactionOutput>
        + Send
        + Sync,
    PM: StateProjectionRead<PermManagerSnapshot, TransactionOutput>
        + StateProjectionWrite<PermManagerSnapshot, TransactionOutput>
        + Send
        + Sync,
    FB: FundingRepo + Send + Sync,
    MVE: ResilientBacklog<MakeVotingEscrowOrderBundle<TransactionOutput>> + Send + Sync,
    OVE: KvStore<Owner, MVEStatus> + Send + Sync,
    TMVE: KvStore<TimedOutputRef, PendingOrder<MakeVotingEscrowOrderBundle<TransactionOutput>>> + Send + Sync,
    Time: NetworkTimeProvider + Send + Sync,
    Actions: InflationActions<TransactionOutput> + Send + Sync,
    Net: Network<Transaction, RejectReasons> + Clone + Sync + Send,
{
    async fn process_ledger_event(&mut self, ev: LedgerTxEvent<TxViewMut>) {
        match ev {
            LedgerTxEvent::TxApplied {
                tx:
                    TxViewMut {
                        hash,
                        inputs,
                        mut outputs,
                        ..
                    },
                slot,
                ..
            } => {
                self.current_slot = Some(slot);

                if self.predicted_tx_backlog.remove(hash).await.is_some() {
                    trace!("Confirmed TX {}, removing from TX tracker", hash);
                }

                let mut epoch_of_eliminated_wpoll = None;
                let last_processed_epoch = self
                    .inflation_box()
                    .await
                    .and_then(|a| a.erased().0.get().last_processed_epoch);

                let mut mve_utxo_owner = None;
                let mut voting_escrow_in_output = None;

                for input in inputs {
                    let input_output_ref = OutputRef::from(input.clone());
                    if mve_utxo_owner.is_none() {
                        let mut orders = self
                            .mve_order_backlog
                            .find_orders(move |e| e.output_ref.output_ref == input_output_ref)
                            .await;
                        if let Some(o) = orders.pop() {
                            assert!(orders.is_empty());
                            mve_utxo_owner = Some((o.order.ve_datum.owner, input_output_ref));
                        }
                    }
                    if let Some(last_processed_epoch) = last_processed_epoch {
                        if epoch_of_eliminated_wpoll.is_none() {
                            for epoch in (0..=last_processed_epoch).rev() {
                                if let Some(stored_output_ref) = self
                                    .weighting_poll
                                    .read(WeightingPollId(epoch))
                                    .await
                                    .map(|a| a.as_erased().version().output_ref)
                                {
                                    if stored_output_ref == input_output_ref {
                                        // The wpoll of this epoch has been consumed in the input.
                                        // To confirm elimination we inspect the outputs of this TX
                                        // below.
                                        epoch_of_eliminated_wpoll = Some(epoch);
                                        break;
                                    }
                                }
                            }
                        }
                    }
                    let funding_id = FundingBoxId::from(input_output_ref);
                    self.funding_box.spend_confirmed(funding_id).await;
                }

                let num_outputs = outputs.len();
                if num_outputs > 0 {
                    let mut ix = num_outputs - 1;
                    while let Some(output) = outputs.pop() {
                        let timed_output_ref = TimedOutputRef {
                            output_ref: OutputRef::new(hash, ix as u64),
                            slot: Slot(slot),
                        };
                        let network_id = self.conf.network_id;
                        let time_millis = slot_to_time_millis(slot, network_id);
                        let current_epoch = slot_to_epoch(slot, self.conf.genesis_time, network_id);
                        let mut ctx = ProcessLedgerEntityContext {
                            behaviour: self,
                            timed_output_ref,
                            current_epoch,
                            wpoll_eliminated: epoch_of_eliminated_wpoll.is_some(),
                        };

                        if mve_utxo_owner.is_some() {
                            if let Some(voting_escrow) =
                                VotingEscrowSnapshot::try_from_ledger(&output.1, &ctx)
                            {
                                let id = voting_escrow.stable_id();
                                voting_escrow_in_output = Some(MVEStatus::SpentToFormVotingEscrow(id));
                            }
                        }

                        if let Some(potentially_eliminated_wpoll_epoch) = epoch_of_eliminated_wpoll {
                            if let Some(wp_snapshot) = WeightingPollSnapshot::try_from_ledger(&output.1, &ctx)
                            {
                                assert_eq!(wp_snapshot.get().epoch, potentially_eliminated_wpoll_epoch);
                                // If wpoll is seen in the output then it can't be an elimination.
                                epoch_of_eliminated_wpoll = None;
                                // Set this following field to false, and redo it to properly confirm
                                // the weighting_poll below.
                                ctx.wpoll_eliminated = false;
                            }
                        }
                        if let Some(entity) = DaoEntitySnapshot::try_from_ledger(&output.1, &ctx) {
                            trace!(
                                "entity found: {:?}, epoch: {}, block_timestamp: {}, EPOCH_LEN: {}, DAO GEN time: {}",
                                entity, current_epoch.0, time_millis, EPOCH_LEN, self.conf.genesis_time.0
                            );
                            self.confirm_entity(Bundled(entity, output.1)).await;
                        }

                        ix = ix.saturating_sub(1);
                    }

                    if let Some((owner, mve_output_ref)) = mve_utxo_owner {
                        // Whether through the creation of a voting_escrow or refunded, this MVE
                        // order is gone.
                        self.mve_order_backlog.remove(mve_output_ref).await;
                        let status = if let Some(status) = voting_escrow_in_output {
                            status
                        } else {
                            MVEStatus::Refunded
                        };
                        self.owner_to_voting_escrow.insert(owner, status).await;
                    }

                    if let Some(eliminated_epoch) = epoch_of_eliminated_wpoll {
                        info!("wpoll (epoch {}) eliminated", eliminated_epoch);
                        let b = self
                            .weighting_poll
                            .read(WeightingPollId(eliminated_epoch))
                            .await
                            .unwrap();

                        let prev_state_id = match &b {
                            AnyMod::Confirmed(traced) => traced.prev_state_id,
                            AnyMod::Predicted(traced) => traced.prev_state_id,
                        };

                        let mut bundle = b.erased();
                        bundle.0.get_mut().eliminated = true;
                        self.weighting_poll
                            .write_confirmed(Traced::new(Confirmed(bundle), prev_state_id))
                            .await;
                    }
                }
            }
            LedgerTxEvent::TxUnapplied {
                tx:
                    TxViewMut {
                        hash,
                        inputs,
                        outputs,
                        ..
                    },
                slot,
                ..
            } => {
                self.current_slot = None;
                for (ix, input) in inputs.into_iter().enumerate() {
                    let timed_output_ref = TimedOutputRef {
                        output_ref: OutputRef::new(hash, ix as u64),
                        slot: Slot(slot),
                    };

                    if let Some(ord) = self.tx_hash_to_mve.get(timed_output_ref).await {
                        // If a `make_voting_escrow_order` was consumed to create a `voting_escrow`, return the order to backlog.
                        self.mve_order_backlog.put(ord).await;
                    } else {
                        let id = FundingBoxId::from(OutputRef::from(input));
                        self.funding_box.unspend_confirmed(id).await;
                    }
                }

                for ix in 0..outputs.len() {
                    let ver = TimedOutputRef {
                        output_ref: OutputRef::new(hash, ix as u64),
                        slot: Slot(slot),
                    };
                    if let Some(id) = self.inflation_box.get_id(ver).await {
                        self.inflation_box.remove(id).await;
                    } else if let Some(id) = self.poll_factory.get_id(ver).await {
                        self.poll_factory.remove(id).await;
                    } else if let Some(id) = self.weighting_poll.get_id(ver).await {
                        self.weighting_poll.remove(id).await;
                    } else if let Some(id) = self.voting_escrow.get_id(ver).await {
                        self.voting_escrow.remove(id).await;
                    } else if let Some(id) = self.smart_farm.get_id(ver).await {
                        self.smart_farm.remove(id).await;
                    } else if let Some(id) = self.perm_manager.get_id(ver).await {
                        self.perm_manager.remove(id).await;
                    } else if let Some(id) = self.ve_factory.get_id(ver).await {
                        self.ve_factory.remove(id).await;
                    } else if self.mve_order_backlog.exists(ver.output_ref).await {
                        self.mve_order_backlog.remove(ver.output_ref).await;
                    } else {
                        self.funding_box
                            .spend_confirmed(FundingBoxId::from(ver.output_ref))
                            .await;
                    }
                }
            }
        }
    }

    async fn processing_dao_bot_message(&mut self, message: DaoBotMessage)
    where
        Backlog: ResilientBacklog<VotingOrder> + Send + Sync,
        Time: NetworkTimeProvider + Send + Sync,
        OVE: KvStore<Owner, MVEStatus> + Send + Sync,
        VE: StateProjectionRead<VotingEscrowSnapshot, TransactionOutput> + Send + Sync,
    {
        let DaoBotMessage {
            command,
            response_sender,
        } = message;
        let send_response_result = match command {
            DaoBotCommand::VotingOrder(VotingOrderCommand::Submit(voting_order)) => {
                if !self.voting_order_backlog.exists(voting_order.id).await {
                    let time_src = NetworkTimeSource {};
                    let timestamp = time_src.network_time().await as i64;
                    let ord = PendingOrder {
                        order: voting_order,
                        timestamp,
                    };
                    self.voting_order_backlog.put(ord).await;
                    Some(response_sender.send(DaoBotResponse::VotingOrder(VotingOrderStatus::Queued)))
                } else {
                    trace!("Order already exists in backlog");
                    None
                }
            }
            DaoBotCommand::VotingOrder(VotingOrderCommand::GetStatus(order_id)) => {
                if self.voting_order_backlog.exists(order_id).await {
                    Some(response_sender.send(DaoBotResponse::VotingOrder(VotingOrderStatus::Queued)))
                } else if let Some(ve) = self.voting_escrow.read(order_id.voting_escrow_id).await {
                    let ve_version = ve.as_erased().0.get().version as u64;
                    if ve_version > order_id.version {
                        Some(response_sender.send(DaoBotResponse::VotingOrder(VotingOrderStatus::Success)))
                    } else {
                        Some(response_sender.send(DaoBotResponse::VotingOrder(VotingOrderStatus::Failed)))
                    }
                } else {
                    Some(response_sender.send(DaoBotResponse::VotingOrder(
                        VotingOrderStatus::VotingEscrowNotFound,
                    )))
                }
            }
            DaoBotCommand::GetMVEOrderStatus { mve_order_owner } => self
                .owner_to_voting_escrow
                .get(mve_order_owner)
                .await
                .map(|status| response_sender.send(DaoBotResponse::MVEStatus(status))),
        };
        if let Some(res) = send_response_result {
            match res {
                Ok(_) => {
                    trace!("Response sent to user");
                }
                Err(_e) => {
                    trace!("Couldn't send response to user");
                }
            }
        }
    }

    async fn revert_bot_action(&mut self, tx_hash: TransactionHash) {
        if let Some(pred_action) = self.predicted_tx_backlog.remove(tx_hash).await {
            match pred_action {
                PredictedEntityWrites::CreateWPoll {
                    inflation_box_id,
                    wp_factory_id,
                    wpoll_id,
                    funding_box_changes,
                    ..
                } => {
                    info!("revert_bot_action(): Create WPoll TX timed out, reverting bot state");
                    trace!("removing inflation box");
                    self.inflation_box.remove(inflation_box_id).await;
                    trace!("removing poll_factory");
                    self.poll_factory.remove(wp_factory_id).await;
                    trace!("removing weighting_poll");
                    self.weighting_poll.remove(wpoll_id).await;

                    for p in funding_box_changes.spent {
                        self.funding_box.unspend_predicted(p).await;
                    }
                }
                PredictedEntityWrites::ApplyVotingOrder {
                    voting_order,
                    wpoll_id,
                    voting_escrow_id,
                    ..
                } => {
                    info!(
                        "revert_bot_action(): Apply voting order {:?} timed out, reverting bot state",
                        voting_order.id
                    );
                    self.weighting_poll.remove(wpoll_id).await;
                    self.voting_escrow.remove(voting_escrow_id).await;
                    let time_src = NetworkTimeSource {};
                    let timestamp = time_src.network_time().await as i64;
                    let ord = PendingOrder {
                        order: voting_order,
                        timestamp,
                    };
                    self.voting_order_backlog.put(ord).await;
                }
                PredictedEntityWrites::DistributeInflation {
                    wpoll_id,
                    smart_farm_id,
                    funding_box_changes,
                    ..
                } => {
                    info!(
                        "revert_bot_action(): Distribute inflation tx (epoch {}) timed out, reverting bot state",
                        wpoll_id.0
                    );
                    self.weighting_poll.remove(wpoll_id).await;
                    self.smart_farm.remove(smart_farm_id).await;

                    for p in funding_box_changes.spent {
                        self.funding_box.unspend_predicted(p).await;
                    }
                }
                PredictedEntityWrites::EiminateWPoll {
                    funding_box_changes, ..
                } => {
                    info!("revert_bot_action(): Eliminate WPOLL timed out, reverting bot state",);
                    // Recall that on wpoll
                    for p in funding_box_changes.spent {
                        self.funding_box.unspend_predicted(p).await;
                    }
                }
                PredictedEntityWrites::MakeVotingEscrow {
                    voting_escrow_id,
                    mve_order,
                    ..
                } => {
                    let time_src = NetworkTimeSource {};
                    let timestamp = time_src.network_time().await as i64;
                    let version = mve_order.output_ref;
                    self.voting_escrow.remove(voting_escrow_id).await;
                    let order = PendingOrder {
                        order: mve_order,
                        timestamp,
                    };
                    self.mve_order_backlog.put(order).await;
                    self.tx_hash_to_mve.remove(version).await;
                }
            }
        }
    }

    #[allow(clippy::needless_lifetimes)]
    pub fn as_stream<'a>(&'a mut self) -> impl Stream<Item = ()> + 'a {
        let chain_tip_reached_clone = self.chain_tip_reached.clone();
        let state_synced = self.state_synced.clone();
        tokio::spawn(async move {
            trace!("wait for signal tip");
            let _ = state_synced.once(true).await;

            let mut reached = chain_tip_reached_clone.lock().await;
            *reached = true;
            trace!("signal tip reached!");
        });
        let mut routine: Option<ToRoutine> = None;
        stream! {

            loop {
                while let Ok(ev) = self.ledger_upstream.try_recv() {
                    self.process_ledger_event(ev).await;
                }

                while let Ok(voting_order_msg) = self.voting_orders.try_recv() {
                    self.processing_dao_bot_message(voting_order_msg).await;
                }

                while let Ok(tx) = self.failed_to_confirm_txs_recv.try_recv() {
                    let tx_hash = tx.body.hash();
                    self.revert_bot_action(tx_hash).await;
                }

                let chain_tip_reached = {
                    *self.chain_tip_reached.lock().await
                };
                if chain_tip_reached {
                    if let Some(r) = routine {
                        match r {
                            ToRoutine::RetryIn(delay) => {
                                Delay::new(delay).await;
                            }
                        }
                    }

                    if let Some(r) = self.attempt().await {
                        routine = Some(r);
                    } else {
                        routine = None;
                    }
                } else {
                    Delay::new(Duration::from_millis(100)).await;
                }
                yield ();
            }
        }
    }
}

#[derive(
    Clone,
    Copy,
    Debug,
    PartialOrd,
    Ord,
    PartialEq,
    Eq,
    serde::Serialize,
    Hash,
    serde::Deserialize,
    derive_more::Display,
)]
pub struct Slot(pub u64);

#[derive(Clone, Debug, PartialOrd, Ord, Copy, PartialEq, Eq, serde::Serialize, Hash, serde::Deserialize)]
pub struct TimedOutputRef {
    pub output_ref: OutputRef,
    pub slot: Slot,
}

impl Display for TimedOutputRef {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(format!("{}, {}", self.output_ref, self.slot).as_str())
    }
}

pub struct ProcessLedgerEntityContext<'a, D> {
    pub behaviour: &'a D,
    pub timed_output_ref: TimedOutputRef,
    pub current_epoch: CurrentEpoch,
    pub wpoll_eliminated: bool,
}

impl<'a, D> Has<OutputRef> for ProcessLedgerEntityContext<'a, D> {
    fn select<U: IsEqual<OutputRef>>(&self) -> OutputRef {
        self.timed_output_ref.output_ref
    }
}

impl<'a, D> Has<TimedOutputRef> for ProcessLedgerEntityContext<'a, D> {
    fn select<U: IsEqual<TimedOutputRef>>(&self) -> TimedOutputRef {
        self.timed_output_ref
    }
}

impl<'a, D> Has<WeightingPollEliminated> for ProcessLedgerEntityContext<'a, D> {
    fn select<U: IsEqual<WeightingPollEliminated>>(&self) -> WeightingPollEliminated {
        WeightingPollEliminated(self.wpoll_eliminated)
    }
}

impl<'a, D> Has<CurrentEpoch> for ProcessLedgerEntityContext<'a, D> {
    fn select<U: IsEqual<CurrentEpoch>>(&self) -> CurrentEpoch {
        self.current_epoch
    }
}

impl<'a, D, H> Has<H> for ProcessLedgerEntityContext<'a, D>
where
    D: Has<H>,
    H: NotOutputRefNorSlotNumber,
{
    fn select<U: IsEqual<H>>(&self) -> H {
        self.behaviour.select::<U>()
    }
}

impl<IB, PF, VEF, WP, VE, SF, PM, FB, MVE, OVE, TMVE, Backlog, PTX, Time, Actions, Net, H> Has<H>
    for Behaviour<
        IB,
        PF,
        VEF,
        WP,
        VE,
        SF,
        PM,
        FB,
        MVE,
        OVE,
        TMVE,
        Backlog,
        PTX,
        Time,
        Actions,
        TransactionOutput,
        Net,
    >
where
    ProtocolConfig: Has<H>,
{
    fn select<U: IsEqual<H>>(&self) -> H {
        self.conf.select::<U>()
    }
}

pub fn slot_to_time_millis(slot: u64, network_id: NetworkId) -> u64 {
    if network_id == NetworkId::from(0) {
        // Preprod
        (1655683200 + slot) * 1000
    } else {
        // Mainnet
        (1596491091 + slot - 4924800) * 1000
    }
}

pub fn time_millis_to_epoch(time_millis: u64, genesis_time: GenesisEpochStartTime) -> CurrentEpoch {
    let diff = if time_millis < genesis_time.0 {
        0.0
    } else {
        (time_millis - genesis_time.0) as f32
    };
    CurrentEpoch((diff / EPOCH_LEN as f32).floor() as u32)
}

pub fn slot_to_epoch(slot: u64, genesis_time: GenesisEpochStartTime, network_id: NetworkId) -> CurrentEpoch {
    let time_millis = slot_to_time_millis(slot, network_id);
    time_millis_to_epoch(time_millis, genesis_time)
}

pub struct WeightingPollEliminated(pub bool);

pub enum EpochRoutineState<Out> {
    /// Protocol wasn't initialized yet.
    Uninitialized,
    /// It's time to create a new WP for epoch `e`
    /// and pour it with epochly emission.
    PendingCreatePoll(PendingCreatePoll<Out>),
    /// Weighting in progress, applying votes from GT holders.
    WeightingInProgress(WeightingInProgress<Out>),
    /// Weighting ended. Time to distribute inflation to farms pro-rata.
    DistributionInProgress(DistributionInProgress<Out>),
    /// Weighting ended. Awaiting start time to distribute inflation to farms pro-rata.
    WaitingForDistributionToStart,
    /// Inflation is distributed, but not yet time to eliminate the wpoll.
    WaitingToEliminate,
    /// Inflation is distributed, and it's time to eliminate the poll.
    PendingEliminatePoll(PendingEliminatePoll<Out>),
    /// Wpoll is eliminated
    Eliminated,
}

pub struct RoutineState<Out> {
    pub previous_epoch_state: Option<EpochRoutineState<Out>>,
    pub current_epoch_state: EpochRoutineState<Out>,
    pub eliminate_wpoll: Option<(PendingEliminatePoll<Out>, ProtocolEpoch)>,
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

#[derive(Clone, PartialEq, Eq, Debug, Hash, serde::Serialize, serde::Deserialize)]
/// Changes to operator funding boxes resulting from inflation action TXs.
pub struct FundingBoxChanges {
    pub spent: Vec<FundingBoxId>,
    pub created: Vec<Predicted<FundingBox>>,
}

#[derive(Debug, Clone)]
pub struct AvailableFundingBoxes(pub Vec<FundingBox>);

pub struct DaoBotMessage {
    pub command: DaoBotCommand,
    pub response_sender: tokio::sync::oneshot::Sender<DaoBotResponse>,
}

#[derive(Debug, Clone)]
pub struct WPollEliminated;

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
enum OnChainStatus {
    Created(OutputRef),
    Eliminated(OutputRef),
}

pub enum DaoBotCommand {
    VotingOrder(VotingOrderCommand),
    GetMVEOrderStatus { mve_order_owner: Owner },
}

pub enum VotingOrderCommand {
    Submit(VotingOrder),
    GetStatus(VotingOrderId),
}

/// Tracks predicted entities that are saved to persistent storage as a result of inflation actions.
/// We need this because TXs can be silently dropped from mempool and so we must remove these
/// entities from storage when that happens.
#[derive(Clone, PartialEq, Eq, Debug, Hash, serde::Serialize, serde::Deserialize)]
pub enum PredictedEntityWrites<Bearer> {
    CreateWPoll {
        inflation_box_id: InflationBoxId,
        wp_factory_id: PollFactoryId,
        wpoll_id: WeightingPollId,
        funding_box_changes: FundingBoxChanges,
        tx_hash: TransactionHash,
    },
    ApplyVotingOrder {
        wpoll_id: WeightingPollId,
        voting_escrow_id: VotingEscrowId,
        voting_order: VotingOrder,
        tx_hash: TransactionHash,
    },
    DistributeInflation {
        wpoll_id: WeightingPollId,
        smart_farm_id: FarmId,
        funding_box_changes: FundingBoxChanges,
        tx_hash: TransactionHash,
    },
    EiminateWPoll {
        funding_box_changes: FundingBoxChanges,
        tx_hash: TransactionHash,
    },
    MakeVotingEscrow {
        tx_hash: TransactionHash,
        voting_escrow_id: VotingEscrowId,
        mve_order: MakeVotingEscrowOrderBundle<Bearer>,
    },
}

impl<Bearer> UniqueOrder for PredictedEntityWrites<Bearer> {
    type TOrderId = TransactionHash;

    fn get_self_ref(&self) -> Self::TOrderId {
        match self {
            PredictedEntityWrites::CreateWPoll { tx_hash, .. }
            | PredictedEntityWrites::ApplyVotingOrder { tx_hash, .. }
            | PredictedEntityWrites::DistributeInflation { tx_hash, .. }
            | PredictedEntityWrites::EiminateWPoll { tx_hash, .. }
            | PredictedEntityWrites::MakeVotingEscrow { tx_hash, .. } => *tx_hash,
        }
    }
}

impl<Bearer> Weighted for PredictedEntityWrites<Bearer> {
    fn weight(&self) -> OrderWeight {
        OrderWeight::from(1)
    }
}

#[derive(Clone, Debug, serde::Serialize)]
pub enum DaoBotResponse {
    VotingOrder(VotingOrderStatus),
    MVEStatus(MVEStatus),
}

#[derive(Clone, Debug, serde::Serialize)]
pub enum VotingOrderStatus {
    Queued,
    /// TODO: should fold in TX hash
    Success,
    /// TODO: give reason for failure
    Failed,
    VotingEscrowNotFound,
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use async_trait::async_trait;
    use cml_chain::PolicyId;
    use cml_crypto::ScriptHash;
    use serde::Serialize;
    use tokio::sync::Mutex;

    use bloom_offchain::execution_engine::bundled::Bundled;
    use spectrum_offchain::domain::{
        event::{AnyMod, Confirmed, Predicted, Traced},
        EntitySnapshot, Identifier,
    };

    use crate::state_projection::{StateProjectionRead, StateProjectionWrite};

    struct StateProjection<T: EntitySnapshot, B>(Arc<Mutex<Option<AnyMod<Bundled<T, B>>>>>);
    #[async_trait]
    impl<T, B> StateProjectionWrite<T, B> for StateProjection<T, B>
    where
        T: EntitySnapshot + Send + Sync + Clone,
        T::Version: Send,
        B: Send + Sync + Clone,
        T::StableId: Send + Serialize + Sync,
    {
        async fn write_predicted(&self, entity: Traced<Predicted<Bundled<T, B>>>) {
            let _ = self.0.lock().await.insert(AnyMod::Predicted(entity));
        }

        async fn write_confirmed(&self, entity: Traced<Confirmed<Bundled<T, B>>>) {
            let _ = self.0.lock().await.insert(AnyMod::Confirmed(entity));
        }

        async fn remove(&self, id: T::StableId) -> Option<T::Version> {
            // Stub
            None
        }
    }
    #[async_trait]
    impl<T, B> StateProjectionRead<T, B> for StateProjection<T, B>
    where
        T: EntitySnapshot + Send + Sync + Clone,
        T::Version: Send,
        B: Send + Sync + Clone,
        T::StableId: Send + Serialize + Sync,
    {
        async fn read(&self, id: T::StableId) -> Option<AnyMod<Bundled<T, B>>> {
            self.0.lock().await.clone()
        }

        async fn get_id(&self, ver: T::Version) -> Option<T::StableId> {
            None
        }
    }
}
