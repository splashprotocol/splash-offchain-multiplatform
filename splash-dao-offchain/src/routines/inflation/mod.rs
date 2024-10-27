use std::collections::VecDeque;
use std::marker::PhantomData;
use std::pin::{pin, Pin};
use std::sync::Arc;
use std::task::Poll;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use actions::ExecuteOrderError;
use async_stream::stream;
use bloom_offchain::execution_engine::bundled::Bundled;
use bloom_offchain_cardano::event_sink::processed_tx::TxViewAtEraBoundary;
use cardano_chain_sync::data::LedgerTxEvent;
use cml_chain::plutus::{PlutusData, PlutusScript, PlutusV2Script};
use cml_chain::transaction::{Transaction, TransactionOutput};
use cml_chain::Serialize;
use cml_crypto::{PrivateKey, RawBytesEncoding, ScriptHash, TransactionHash};
use cml_multi_era::babbage::BabbageTransaction;
use futures::{pin_mut, Future, FutureExt, Stream, StreamExt};
use futures_timer::Delay;
use log::{error, info};
use pallas_network::miniprotocols::localtxsubmission::cardano_node_errors::{
    ApplyTxError, ConwayLedgerPredFailure, ConwayUtxoPredFailure, ConwayUtxowPredFailure,
};
use spectrum_cardano_lib::output::FinalizedTxOut;
use spectrum_cardano_lib::transaction::{BabbageTransactionOutputExtension, OutboundTransaction};
use spectrum_cardano_lib::{AssetName, OutputRef};
use spectrum_offchain::backlog::ResilientBacklog;
use spectrum_offchain::data::event::{AnyMod, Confirmed, Predicted, Traced, Unconfirmed};
use spectrum_offchain::data::order::PendingOrder;
use spectrum_offchain::data::{EntitySnapshot, Has};
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

use crate::constants::script_bytes::VOTING_WITNESS_STUB;
use crate::deployment::ProtocolValidator;
use crate::entities::offchain::voting_order::{VotingOrder, VotingOrderId};
use crate::entities::onchain::funding_box::{FundingBox, FundingBoxId, FundingBoxSnapshot};
use crate::entities::onchain::inflation_box::{InflationBoxId, InflationBoxSnapshot};
use crate::entities::onchain::permission_manager::{PermManager, PermManagerId, PermManagerSnapshot};
use crate::entities::onchain::poll_factory::{PollFactory, PollFactoryId, PollFactorySnapshot};
use crate::entities::onchain::smart_farm::{FarmId, SmartFarm, SmartFarmSnapshot};
use crate::entities::onchain::voting_escrow::{VotingEscrow, VotingEscrowId, VotingEscrowSnapshot};
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
use crate::time::{NetworkTimeProvider, ProtocolEpoch};
use crate::{CurrentEpoch, NetworkTimeSource};

pub mod actions;

pub struct Behaviour<IB, PF, WP, VE, SF, PM, FB, Backlog, Time, Actions, Bearer, Net> {
    inflation_box: IB,
    poll_factory: PF,
    weighting_poll: WP,
    voting_escrow: VE,
    smart_farm: SF,
    perm_manager: PM,
    funding_box: FB,
    backlog: Backlog,
    ntp: Time,
    actions: Actions,
    pub conf: ProtocolConfig,
    pd: PhantomData<Bearer>,
    network: Net,
    operator_sk: PrivateKey,
    ledger_upstream: Receiver<LedgerTxEvent<TxViewAtEraBoundary>>,
    voting_orders: Receiver<VotingOrderMessage>,
    chain_tip_reached: Arc<Mutex<bool>>,
    signal_tip_reached_recv: Option<tokio::sync::broadcast::Receiver<bool>>,
    current_slot: u64,
}

const DEF_DELAY: Duration = Duration::new(5, 0);

#[async_trait::async_trait]
impl<IB, PF, WP, VE, SF, PM, FB, Backlog, Time, Actions, Bearer, Net> RoutineBehaviour
    for Behaviour<IB, PF, WP, VE, SF, PM, FB, Backlog, Time, Actions, Bearer, Net>
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
    FB: FundingRepo + Send + Sync,
    Time: NetworkTimeProvider + Send + Sync,
    Actions: InflationActions<Bearer> + Send + Sync,
    Bearer: Send + Sync + std::fmt::Debug,
    Net: Network<OutboundTransaction<Transaction>, RejectReasons> + Clone + Sync + Send,
{
    async fn attempt(&mut self) -> Option<ToRoutine> {
        match self.read_state().await {
            RoutineState::Uninitialized => {
                println!("UNINIT");
                retry_in(DEF_DELAY)
            }
            RoutineState::PendingCreatePoll(state) => {
                println!("PendingCreatePoll");
                self.try_create_wpoll(state).await
            }
            RoutineState::WeightingInProgress(state) => self.try_apply_votes(state).await,
            RoutineState::DistributionInProgress(state) => {
                self.try_distribute_inflation(state).await;
                None
            }
            RoutineState::PendingEliminatePoll(state) => self.try_eliminate_poll(state).await,
        }
    }
}

impl<IB, PF, WP, VE, SF, PM, FB, Backlog, Time, Actions, Bearer, Net>
    Behaviour<IB, PF, WP, VE, SF, PM, FB, Backlog, Time, Actions, Bearer, Net>
{
    pub fn new(
        inflation_box: IB,
        poll_factory: PF,
        weighting_poll: WP,
        voting_escrow: VE,
        smart_farm: SF,
        perm_manager: PM,
        funding_box: FB,
        backlog: Backlog,
        ntp: Time,
        actions: Actions,
        conf: ProtocolConfig,
        pd: PhantomData<Bearer>,
        network: Net,
        operator_sk: PrivateKey,
        ledger_upstream: Receiver<LedgerTxEvent<TxViewAtEraBoundary>>,
        voting_orders: Receiver<VotingOrderMessage>,
        signal_tip_reached_recv: tokio::sync::broadcast::Receiver<bool>,
    ) -> Self
    where
        Backlog: ResilientBacklog<VotingOrder> + Send + Sync,
    {
        Self {
            inflation_box,
            poll_factory,
            weighting_poll,
            voting_escrow,
            smart_farm,
            perm_manager,
            funding_box,
            backlog,
            ntp,
            actions,
            conf,
            pd,
            network,
            operator_sk,
            ledger_upstream,
            voting_orders,
            chain_tip_reached: Arc::new(Mutex::new(false)),
            signal_tip_reached_recv: Some(signal_tip_reached_recv),
            current_slot: 0,
        }
    }

    async fn inflation_box(&self) -> Option<AnyMod<Bundled<InflationBoxSnapshot, Bearer>>>
    where
        IB: StateProjectionRead<InflationBoxSnapshot, Bearer> + Send + Sync,
    {
        self.inflation_box.read(InflationBoxId).await
    }

    pub async fn get_current_epoch(&self) -> CurrentEpoch
    where
        IB: StateProjectionRead<InflationBoxSnapshot, Bearer> + Send + Sync,
    {
        if let Some(m) = self.inflation_box().await {
            let time_src = NetworkTimeSource {};
            let now_millis = time_src.network_time().await * 1000;
            match m {
                AnyMod::Confirmed(Traced {
                    state: Confirmed(Bundled(snapshot, _)),
                    ..
                }) => CurrentEpoch(
                    snapshot
                        .get()
                        .active_epoch(self.conf.genesis_time.into(), now_millis),
                ),
                AnyMod::Predicted(Traced {
                    state: Predicted(Bundled(snapshot, _)),
                    ..
                }) => {
                    let predicted_active_epoch = snapshot
                        .get()
                        .active_epoch(self.conf.genesis_time.into(), now_millis);
                    if predicted_active_epoch > 0 {
                        CurrentEpoch(predicted_active_epoch - 1)
                    } else {
                        CurrentEpoch(0)
                    }
                }
            }
        } else {
            CurrentEpoch(0)
        }
    }

    async fn poll_factory(&self) -> Option<AnyMod<Bundled<PollFactorySnapshot, Bearer>>>
    where
        PF: StateProjectionRead<PollFactorySnapshot, Bearer> + Send + Sync,
    {
        self.poll_factory.read(PollFactoryId).await
    }

    async fn weighting_poll(
        &self,
        epoch: ProtocolEpoch,
    ) -> Option<AnyMod<Bundled<WeightingPollSnapshot, Bearer>>>
    where
        WP: StateProjectionRead<WeightingPollSnapshot, Bearer> + Send + Sync,
    {
        self.weighting_poll.read(self.conf.poll_id(epoch)).await
    }

    async fn perm_manager(&self) -> Option<AnyMod<Bundled<PermManagerSnapshot, Bearer>>>
    where
        PM: StateProjectionRead<PermManagerSnapshot, Bearer> + Send + Sync,
    {
        self.perm_manager.read(PermManagerId {}).await
    }

    async fn next_order(
        &mut self,
        _stage: WeightingOngoing,
    ) -> Option<(VotingOrder, Bundled<VotingEscrowSnapshot, Bearer>)>
    where
        VE: StateProjectionRead<VotingEscrowSnapshot, Bearer> + Send + Sync,
        Backlog: ResilientBacklog<VotingOrder> + Send + Sync,
        Bearer: std::fmt::Debug,
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

    async fn read_state(&mut self) -> RoutineState<Bearer>
    where
        IB: StateProjectionRead<InflationBoxSnapshot, Bearer> + Send + Sync,
        PF: StateProjectionRead<PollFactorySnapshot, Bearer> + Send + Sync,
        PM: StateProjectionRead<PermManagerSnapshot, Bearer> + Send + Sync,
        WP: StateProjectionRead<WeightingPollSnapshot, Bearer> + Send + Sync,
        VE: StateProjectionRead<VotingEscrowSnapshot, Bearer> + Send + Sync,
        SF: StateProjectionRead<SmartFarmSnapshot, Bearer> + Send + Sync,
        Backlog: ResilientBacklog<VotingOrder> + Send + Sync,
        Bearer: std::fmt::Debug,
        Time: NetworkTimeProvider + Send + Sync,
    {
        let ibox = self.inflation_box().await;
        let wp_factory = self.poll_factory().await;
        println!("{:?}\n\n{:?}", ibox.is_some(), wp_factory.is_some());
        if let (Some(inflation_box), Some(poll_factory)) = (
            //Some(perm_manager)) = (
            ibox, wp_factory,
            //self.perm_manager().await,
        ) {
            let genesis = self.conf.genesis_time;
            let now_millis = self.ntp.network_time().await * 1000;
            let current_epoch = inflation_box
                .as_erased()
                .0
                .get()
                .active_epoch(genesis, now_millis);
            println!("read_state: current_epoch: {}", current_epoch);
            match self.weighting_poll(current_epoch).await {
                None => {
                    println!("self.weighting_poll(current_epoch) == None");
                    RoutineState::PendingCreatePoll(PendingCreatePoll {
                        inflation_box,
                        poll_factory,
                    })
                }
                Some(wp) => match wp.as_erased().0.get().state(genesis, now_millis) {
                    PollState::WeightingOngoing(st) => {
                        println!("WeightingOnGoing");
                        RoutineState::WeightingInProgress(WeightingInProgress {
                            weighting_poll: wp,
                            next_pending_order: self.next_order(st).await,
                        })
                    }
                    PollState::DistributionOngoing(next_farm) => {
                        todo!()
                        //RoutineState::DistributionInProgress(DistributionInProgress {
                        //    next_farm: self
                        //        .smart_farm
                        //        .read(next_farm.farm_id())
                        //        .await
                        //        .expect("State is inconsistent"),
                        //    weighting_poll: wp,
                        //    next_farm_weight: next_farm.farm_weight(),
                        //    perm_manager,
                        //})
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
        FB: FundingRepo + Send + Sync,
    {
        match entity.get() {
            DaoEntity::Inflation(ib) => {
                let confirmed_snapshot = Confirmed(Bundled(Snapshot::new(*ib, *entity.version()), bearer));
                let prev_state_id = if let Some(state) = self.inflation_box.read(InflationBoxId).await {
                    let bundled = state.erased();
                    Some(bundled.version())
                } else {
                    None
                };
                let traced = Traced {
                    state: confirmed_snapshot,
                    prev_state_id,
                };
                self.inflation_box.write_confirmed(traced).await;
                assert!(self.inflation_box().await.is_some());
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
                let prev_state_id = if let Some(state) = self.poll_factory.read(PollFactoryId).await {
                    let bundled = state.erased();
                    Some(bundled.version())
                } else {
                    None
                };
                let traced = Traced {
                    state: confirmed_snapshot,
                    prev_state_id,
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
                    .read(VotingEscrowId::from(ve.ve_identifier_policy))
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
                self.weighting_poll.write_confirmed(traced).await;
            }

            DaoEntity::FundingBox(fb) => {
                self.funding_box.put_confirmed(Confirmed(fb.clone())).await;
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
        Net: Network<OutboundTransaction<Transaction>, RejectReasons> + Clone + Sync + Send,
        IB: StateProjectionWrite<InflationBoxSnapshot, Bearer> + Send + Sync,
        PF: StateProjectionWrite<PollFactorySnapshot, Bearer> + Send + Sync,
        WP: StateProjectionWrite<WeightingPollSnapshot, Bearer> + Send + Sync,
        FB: FundingRepo + Send + Sync,
    {
        println!("try_create_wpoll");
        if let (AnyMod::Confirmed(inflation_box), AnyMod::Confirmed(factory)) = (inflation_box, poll_factory)
        {
            println!("try_create_wpoll: confirmed!");
            let funding_boxes = AvailableFundingBoxes(self.funding_box.collect().await.unwrap());
            let (signed_tx, next_inflation_box, next_factory, next_wpoll, funding_box_changes) = self
                .actions
                .create_wpoll(
                    inflation_box.state.0,
                    factory.state.0,
                    Slot(self.current_slot),
                    funding_boxes,
                )
                .await;
            println!("try_create_wpoll: formed wpoll TX!");
            let prover = OperatorProver::new(self.operator_sk.to_bech32());
            let tx = prover.prove(signed_tx);
            println!("try_create_wpoll: wpoll TX signed!");
            self.network.submit_tx(tx).await.unwrap();
            println!("try_create_wpoll: wpoll TX submitted!");
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
        Net: Network<OutboundTransaction<Transaction>, RejectReasons> + Clone + Sync + Send,
        WP: StateProjectionWrite<WeightingPollSnapshot, Bearer> + Send + Sync,
        VE: StateProjectionWrite<VotingEscrowSnapshot, Bearer> + Send + Sync,
        Backlog: ResilientBacklog<VotingOrder> + Send + Sync,
        FB: FundingRepo + Send + Sync,
    {
        if let Some(next_order) = next_pending_order {
            let order = next_order.0.clone();
            let order_id = order.id;
            if let Some((signed_tx, next_wpoll, next_ve)) = self
                .actions
                .execute_order(weighting_poll.erased(), next_order, Slot(self.current_slot))
                .await
            {
                let prover = OperatorProver::new(self.operator_sk.to_bech32());
                let tx = prover.prove(signed_tx);
                match self.network.submit_tx(tx).await {
                    Ok(()) => {
                        self.weighting_poll.write_predicted(next_wpoll).await;
                        self.voting_escrow.write_predicted(next_ve).await;
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
                                ConwayLedgerPredFailure::UtxowFailure(ConwayUtxowPredFailure::UtxoFailure(
                                    ConwayUtxoPredFailure::BadInputsUtxo(_)
                                ),)
                            )
                        }) {
                            info!("`execute_order` TX failed on bad/missing input error");
                            self.backlog.suspend(order).await;
                            return None;
                        } else {
                            // For all other errors we discard the order.
                            error!("TX submit failed on unknown error");
                            self.backlog.remove(order_id).await;
                            return None;
                        }
                    }
                    Err(RejectReasons(None)) => {
                        error!("TX submit failed on unknown error");
                        self.backlog.remove(order_id).await;
                        return None;
                    }
                }
            } else {
                // Here the order has been deemed inadmissible and so it will be removed.
                self.backlog.remove(order_id).await;
                return None;
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
    ) where
        Actions: InflationActions<Bearer> + Send + Sync,
        Net: Network<OutboundTransaction<Transaction>, RejectReasons> + Clone + Sync + Send,
        WP: StateProjectionWrite<WeightingPollSnapshot, Bearer> + Send + Sync,
        SF: StateProjectionWrite<SmartFarmSnapshot, Bearer> + Send + Sync,
        PM: StateProjectionWrite<PermManagerSnapshot, Bearer> + Send + Sync,
        FB: FundingRepo + Send + Sync,
    {
        let funding_boxes = AvailableFundingBoxes(self.funding_box.collect().await.unwrap());
        let (signed_tx, next_wpoll, next_sf, next_pm) = self
            .actions
            .distribute_inflation(
                weighting_poll.erased(),
                next_farm.erased(),
                perm_manager.erased(),
                next_farm_weight,
                funding_boxes,
            )
            .await;
        let prover = OperatorProver::new(self.operator_sk.to_bech32());
        let tx = prover.prove(signed_tx);
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
        Actions: InflationActions<Bearer> + Send + Sync,
        Net: Network<OutboundTransaction<Transaction>, RejectReasons> + Clone + Sync + Send,
        FB: FundingRepo + Send + Sync,
    {
        if let AnyMod::Confirmed(Traced {
            state: Confirmed(weighting_poll),
            ..
        }) = weighting_poll
        {
            let funding_boxes = AvailableFundingBoxes(self.funding_box.collect().await.unwrap());
            let signed_tx = self.actions.eliminate_wpoll(weighting_poll, funding_boxes).await;
            let prover = OperatorProver::new(self.operator_sk.to_bech32());
            let tx = prover.prove(signed_tx);
            self.network.submit_tx(tx).await.unwrap();
            return None;
        }
        retry_in(DEF_DELAY)
    }
}

impl<IB, PF, WP, VE, SF, PM, FB, Backlog, Time, Actions, Net>
    Behaviour<IB, PF, WP, VE, SF, PM, FB, Backlog, Time, Actions, TransactionOutput, Net>
where
    IB: StateProjectionRead<InflationBoxSnapshot, TransactionOutput>
        + StateProjectionWrite<InflationBoxSnapshot, TransactionOutput>
        + Send
        + Sync,
    PF: StateProjectionRead<PollFactorySnapshot, TransactionOutput>
        + StateProjectionWrite<PollFactorySnapshot, TransactionOutput>
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
    SF: StateProjectionRead<SmartFarmSnapshot, TransactionOutput>
        + StateProjectionWrite<SmartFarmSnapshot, TransactionOutput>
        + Send
        + Sync,
    PM: StateProjectionRead<PermManagerSnapshot, TransactionOutput>
        + StateProjectionWrite<PermManagerSnapshot, TransactionOutput>
        + Send
        + Sync,
    FB: FundingRepo + Send + Sync,
    Time: NetworkTimeProvider + Send + Sync,
    Actions: InflationActions<TransactionOutput> + Send + Sync,
    Net: Network<OutboundTransaction<Transaction>, RejectReasons> + Clone + Sync + Send,
{
    async fn process_ledger_event(&mut self, ev: LedgerTxEvent<TxViewAtEraBoundary>) {
        match ev {
            LedgerTxEvent::TxApplied {
                tx:
                    TxViewAtEraBoundary {
                        hash,
                        inputs,
                        mut outputs,
                    },
                slot,
            } => {
                self.current_slot = slot;
                let num_outputs = outputs.len();
                if num_outputs > 0 {
                    let mut ix = num_outputs - 1;
                    while let Some(output) = outputs.pop() {
                        let output_ref = OutputRef::new(hash, ix as u64);
                        let current_epoch = self.get_current_epoch().await;
                        let ctx = WithOutputRef {
                            behaviour: self,
                            output_ref,
                            current_epoch,
                        };
                        if let Some(entity) = DaoEntitySnapshot::try_from_ledger(&output.1, &ctx) {
                            println!("entity found: {:?}", entity);
                            self.confirm_entity(Bundled(entity, output.1)).await;
                        }

                        ix = ix.saturating_sub(1);
                    }
                }

                for input in inputs {
                    let id = FundingBoxId::from(OutputRef::from(input));
                    self.funding_box.spend_confirmed(id).await;
                }
            }
            LedgerTxEvent::TxUnapplied(TxViewAtEraBoundary {
                hash,
                inputs,
                outputs,
            }) => {
                for ix in 0..outputs.len() {
                    let ver = OutputRef::new(hash, ix as u64);
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
                    } else {
                        self.funding_box.spend_confirmed(FundingBoxId::from(ver)).await;
                    }
                }

                for input in inputs {
                    let id = FundingBoxId::from(OutputRef::from(input));
                    self.funding_box.unspend_confirmed(id).await;
                }
            }
        }
    }

    async fn processing_voting_order_message(&mut self, message: VotingOrderMessage)
    where
        Backlog: ResilientBacklog<VotingOrder> + Send + Sync,
        Time: NetworkTimeProvider + Send + Sync,
        VE: StateProjectionRead<VotingEscrowSnapshot, TransactionOutput> + Send + Sync,
    {
        let VotingOrderMessage {
            command,
            response_sender,
        } = message;
        match command {
            VotingOrderCommand::Submit(voting_order) => {
                if !self.backlog.exists(voting_order.id).await {
                    let time_src = NetworkTimeSource {};
                    let timestamp = time_src.network_time().await as i64;
                    let ord = PendingOrder {
                        order: voting_order,
                        timestamp,
                    };
                    self.backlog.put(ord).await;
                    response_sender.send(VotingOrderStatus::Queued).unwrap();
                }
            }
            VotingOrderCommand::GetStatus(order_id) => {
                if self.backlog.exists(order_id).await {
                    response_sender.send(VotingOrderStatus::Queued).unwrap();
                } else if let Some(ve) = self.voting_escrow.read(order_id.voting_escrow_id).await {
                    let ve_version = ve.as_erased().0.get().version as u64;
                    if ve_version > order_id.version {
                        response_sender.send(VotingOrderStatus::Success).unwrap();
                    } else {
                        response_sender.send(VotingOrderStatus::Failed).unwrap();
                    }
                } else {
                    response_sender
                        .send(VotingOrderStatus::VotingEscrowNotFound)
                        .unwrap();
                }
            }
        }
    }

    #[allow(clippy::needless_lifetimes)]
    pub fn as_stream<'a>(&'a mut self) -> impl Stream<Item = ()> + 'a {
        let mut signal = self.signal_tip_reached_recv.take().unwrap();
        let chain_tip_reached_clone = self.chain_tip_reached.clone();
        tokio::spawn(async move {
            println!("wait for signal tip");
            let _ = signal.recv().await;

            let mut reached = chain_tip_reached_clone.lock().await;
            *reached = true;
            println!("signal tip reached!");
        });
        let mut routine: Option<ToRoutine> = None;
        stream! {
            loop {
                while let Ok(ev) = self.ledger_upstream.try_recv() {
                    self.process_ledger_event(ev).await;
                }

                while let Ok(voting_order_msg) = self.voting_orders.try_recv() {
                    self.processing_voting_order_message(voting_order_msg).await;
                }

                let chain_tip_reached = {
                    *self.chain_tip_reached.lock().await
                };
                if chain_tip_reached {
                    if let Some(r) = routine {
                        match r {
                            ToRoutine::RetryIn(delay) => {
                                println!("Delay for {:?}", delay);
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

#[derive(Clone, Copy)]
pub struct Slot(pub u64);

pub struct WithOutputRef<'a, D> {
    pub behaviour: &'a D,
    pub output_ref: OutputRef,
    pub current_epoch: CurrentEpoch,
}

impl<'a, D> Has<OutputRef> for WithOutputRef<'a, D> {
    fn select<U: IsEqual<OutputRef>>(&self) -> OutputRef {
        self.output_ref
    }
}

impl<'a, D> Has<CurrentEpoch> for WithOutputRef<'a, D> {
    fn select<U: IsEqual<CurrentEpoch>>(&self) -> CurrentEpoch {
        self.current_epoch
    }
}

impl<'a, D, H> Has<H> for WithOutputRef<'a, D>
where
    D: Has<H>,
    H: NotOutputRefNorSlotNumber,
{
    fn select<U: IsEqual<H>>(&self) -> H {
        self.behaviour.select::<U>()
    }
}

impl<IB, PF, WP, VE, SF, PM, FB, Backlog, Time, Actions, Net, H> Has<H>
    for Behaviour<IB, PF, WP, VE, SF, PM, FB, Backlog, Time, Actions, TransactionOutput, Net>
where
    ProtocolConfig: Has<H>,
{
    fn select<U: IsEqual<H>>(&self) -> H {
        self.conf.select::<U>()
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

/// Changes to operator funding boxes resulting from inflation action TXs.
pub struct FundingBoxChanges {
    pub spent: Vec<FundingBoxId>,
    pub created: Vec<Predicted<FundingBox>>,
}

#[derive(Debug, Clone)]
pub struct AvailableFundingBoxes(pub Vec<FundingBox>);

pub struct VotingOrderMessage {
    pub command: VotingOrderCommand,
    pub response_sender: tokio::sync::oneshot::Sender<VotingOrderStatus>,
}

pub enum VotingOrderCommand {
    Submit(VotingOrder),
    GetStatus(VotingOrderId),
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
    use serde::Serialize;
    use tokio::sync::Mutex;

    use bloom_offchain::execution_engine::bundled::Bundled;
    use spectrum_offchain::data::{
        event::{AnyMod, Confirmed, Predicted, Traced},
        EntitySnapshot, HasIdentifier, Identifier,
    };

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

        async fn get_id(&self, ver: T::Version) -> Option<T::Id> {
            None
        }
    }
}
