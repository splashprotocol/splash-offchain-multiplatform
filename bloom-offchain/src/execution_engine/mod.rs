use std::collections::{BTreeSet, HashSet};
use std::fmt::{Debug, Display};
use std::hash::Hash;
use std::marker::PhantomData;
use std::mem;
use std::pin::Pin;
use std::task::{Context, Poll};

use either::Either;
use futures::channel::mpsc;
use futures::stream::FusedStream;
use futures::{FutureExt, Stream};
use futures::{SinkExt, StreamExt};
use log::{trace, warn};
use tokio::sync::broadcast;

use liquidity_book::interpreter::RecipeInterpreter;
use liquidity_book::stashing_option::StashingOption;
use spectrum_offchain::backlog::HotBacklog;
use spectrum_offchain::circular_filter::CircularFilter;
use spectrum_offchain::combinators::Ior;
use spectrum_offchain::data::event::{Channel, Confirmed, Predicted, StateUpdate, Unconfirmed};
use spectrum_offchain::data::order::{OrderUpdate, SpecializedOrder};
use spectrum_offchain::data::{Baked, EntitySnapshot, Has, Stable};
use spectrum_offchain::maker::Maker;
use spectrum_offchain::network::Network;
use spectrum_offchain::tx_hash::CanonicalHash;
use spectrum_offchain::tx_prover::TxProver;

use crate::execution_engine::backlog::SpecializedInterpreter;
use crate::execution_engine::bundled::Bundled;
use crate::execution_engine::execution_effect::ExecutionEff;
use crate::execution_engine::focus_set::FocusSet;
use crate::execution_engine::funding_effect::{FundingEvent, FundingIO};
use crate::execution_engine::liquidity_book::core::ExecutionRecipe;
use crate::execution_engine::liquidity_book::fragment::MarketTaker;
use crate::execution_engine::liquidity_book::interpreter::ExecutionResult;
use crate::execution_engine::liquidity_book::{ExternalTLBEvents, TLBFeedback, TemporalLiquidityBook};
use crate::execution_engine::multi_pair::MultiPair;
use crate::execution_engine::resolver::resolve_source_state;
use crate::execution_engine::storage::kv_store::KvStore;
use crate::execution_engine::storage::StateIndex;

pub mod backlog;
pub mod batch_exec;
pub mod bundled;
pub mod execution_effect;
mod focus_set;
pub mod funding_effect;
pub mod liquidity_book;
pub mod multi_pair;
pub mod partial_fill;
pub mod resolver;
pub mod storage;
pub mod types;

/// Class of entities that evolve upon execution.
type EvolvingEntity<CO, P, V, B> = Bundled<Either<Baked<CO, V>, Baked<P, V>>, B>;

pub type Event<CO, SO, P, B, V> =
    Either<Channel<StateUpdate<EvolvingEntity<CO, P, V, B>>>, Channel<OrderUpdate<Bundled<SO, B>, SO>>>;

enum ExecutionEffects<CompOrd, SpecOrd, Pool, Ver, Bearer> {
    FromLiquidityBook(
        Vec<
            ExecutionEff<
                EvolvingEntity<CompOrd, Pool, Ver, Bearer>,
                EvolvingEntity<CompOrd, Pool, Ver, Bearer>,
            >,
        >,
    ),
    FromBacklog(Bundled<Baked<Pool, Ver>, Bearer>, Bundled<SpecOrd, Bearer>),
}

struct ExecutionEffectsByPair<Pair, TxHash, CompOrd, SpecOrd, Pool, Ver, Bearer> {
    pair: Pair,
    tx_hash: TxHash,
    consumed_versions: HashSet<Ver>,
    pending_effects: ExecutionEffects<CompOrd, SpecOrd, Pool, Ver, Bearer>,
}

enum Effects<Pair, TxHash, CompOrd, SpecOrd, Pool, Ver, Bearer> {
    Pair(ExecutionEffectsByPair<Pair, TxHash, CompOrd, SpecOrd, Pool, Ver, Bearer>),
    Funding(Vec<FundingEvent<Bearer>>),
}

/// Instantiate execution stream partition.
/// Each partition serves total_pairs/num_partitions pairs.
pub fn execution_part_stream<
    'a,
    Upstream,
    Funding,
    Pair,
    StableId,
    Ver,
    CompOrd,
    SpecOrd,
    Pool,
    Bearer,
    TxCandidate,
    Tx,
    TxHash,
    Ctx,
    MakerCtx,
    ExUnits,
    Index,
    Cache,
    Book,
    Backlog,
    RecInterpreter,
    SpecInterpreter,
    Prover,
    Net,
    Err,
>(
    index: Index,
    cache: Cache,
    book: MultiPair<Pair, Book, MakerCtx>,
    backlog: MultiPair<Pair, Backlog, MakerCtx>,
    context: Ctx,
    rec_interpreter: RecInterpreter,
    spec_interpreter: SpecInterpreter,
    prover: Prover,
    upstream: Upstream,
    funding: Funding,
    network: Net,
    mut tip_reached_signal: broadcast::Receiver<bool>,
) -> impl Stream<Item = ()> + 'a
where
    Upstream: Stream<Item = (Pair, Event<CompOrd, SpecOrd, Pool, Bearer, Ver>)> + Unpin + 'a,
    Funding: Stream<Item = FundingEvent<Bearer>> + Unpin + 'a,
    Pair: Copy + Eq + Ord + Hash + Display + Unpin + 'a,
    StableId: Copy + Eq + Hash + Debug + Display + Unpin + 'a,
    Ver: Copy + Eq + Hash + Display + Unpin + 'a,
    Pool: Stable<StableId = StableId> + Copy + Debug + Unpin + Display + 'a,
    CompOrd: Stable<StableId = StableId> + MarketTaker<U = ExUnits> + Copy + Debug + Unpin + Display + 'a,
    SpecOrd: SpecializedOrder<TPoolId = StableId, TOrderId = Ver> + Debug + Unpin + 'a,
    Bearer: Has<Ver> + Eq + Ord + Clone + Debug + Unpin + 'a,
    TxCandidate: Unpin + 'a,
    Tx: CanonicalHash<Hash = TxHash> + Unpin + 'a,
    TxHash: Display + Unpin + 'a,
    Ctx: Clone + Unpin + 'a,
    MakerCtx: Clone + Unpin + 'a,
    Index: StateIndex<EvolvingEntity<CompOrd, Pool, Ver, Bearer>> + Unpin + 'a,
    Cache: KvStore<StableId, EvolvingEntity<CompOrd, Pool, Ver, Bearer>> + Unpin + 'a,
    Book: TemporalLiquidityBook<CompOrd, Pool>
        + ExternalTLBEvents<CompOrd, Pool>
        + TLBFeedback<CompOrd, Pool>
        + Maker<MakerCtx>
        + Unpin
        + 'a,
    Backlog: HotBacklog<Bundled<SpecOrd, Bearer>> + Maker<MakerCtx> + Unpin + 'a,
    RecInterpreter: RecipeInterpreter<CompOrd, Pool, Ctx, Ver, Bearer, TxCandidate> + Unpin + 'a,
    SpecInterpreter: SpecializedInterpreter<Pool, SpecOrd, Ver, TxCandidate, Bearer, Ctx> + Unpin + 'a,
    Prover: TxProver<TxCandidate, Tx> + Unpin + 'a,
    Net: Network<Tx, Err> + Clone + 'a,
    Err: TryInto<HashSet<Ver>> + Clone + Unpin + Debug + Display + 'a,
{
    let (feedback_out, feedback_in) = mpsc::channel(100);
    let executor = Executor::new(
        index,
        cache,
        book,
        backlog,
        context,
        rec_interpreter,
        spec_interpreter,
        prover,
        upstream,
        funding,
        feedback_in,
    );
    let wait_signal = async move {
        let _ = tip_reached_signal.recv().await;
    };
    wait_signal
        .map(move |_| {
            executor.then(move |tx| {
                let mut network = network.clone();
                let mut feedback = feedback_out.clone();
                async move {
                    let result = network.submit_tx(tx).await;
                    feedback.send(result).await.expect("Filed to propagate feedback.");
                }
            })
        })
        .flatten_stream()
}

pub struct Executor<
    Upstream,
    Funding,
    Pair,
    StableId,
    Ver,
    CompOrd,
    SpecOrd,
    Pool,
    Bearer,
    TxCandidate,
    Tx,
    TxHash,
    Ctx,
    MakerCtx,
    Index,
    Cache,
    Book,
    Backlog,
    TradeInterpreter,
    SpecInterpreter,
    Prover,
    Err,
> {
    /// Storage for all on-chain states.
    index: Index,
    /// Hot storage for resolved states.
    cache: Cache,
    /// Separate TLBs for each pair (for swaps).
    multi_book: MultiPair<Pair, Book, MakerCtx>,
    /// Separate Backlogs for each pair (for specialized operations such as Deposit/Redeem)
    multi_backlog: MultiPair<Pair, Backlog, MakerCtx>,
    context: Ctx,
    trade_interpreter: TradeInterpreter,
    spec_interpreter: SpecInterpreter,
    prover: Prover,
    upstream: Upstream,
    funding_events: Funding,
    funding_pool: BTreeSet<Bearer>,
    /// Feedback channel is used to signal the status of transaction submitted earlier by the executor.
    feedback: mpsc::Receiver<Result<(), Err>>,
    /// Pending effects resulted from execution of a batch trade in a certain [Pair].
    pending_effects: Vec<Effects<Pair, TxHash, CompOrd, SpecOrd, Pool, Ver, Bearer>>,
    /// Which pair should we process in the first place.
    focus_set: FocusSet<Pair>,
    /// Temporarily memoize entities that came from unconfirmed updates.
    skip_filter: CircularFilter<256, Ver>,
    pd: PhantomData<(StableId, Ver, TxCandidate, Tx, Err)>,
}

impl<S, F, PR, SID, V, CO, SO, P, B, TC, TX, TH, C, MC, IX, CH, TLB, L, RIR, SIR, PRV, E>
    Executor<S, F, PR, SID, V, CO, SO, P, B, TC, TX, TH, C, MC, IX, CH, TLB, L, RIR, SIR, PRV, E>
{
    fn new(
        index: IX,
        cache: CH,
        multi_book: MultiPair<PR, TLB, MC>,
        multi_backlog: MultiPair<PR, L, MC>,
        context: C,
        trade_interpreter: RIR,
        spec_interpreter: SIR,
        prover: PRV,
        upstream: S,
        funding_events: F,
        feedback: mpsc::Receiver<Result<(), E>>,
    ) -> Self {
        Self {
            index,
            cache,
            multi_book,
            multi_backlog,
            context,
            trade_interpreter,
            spec_interpreter,
            prover,
            upstream,
            funding_events,
            funding_pool: BTreeSet::new(),
            feedback,
            pending_effects: Vec::new(),
            focus_set: FocusSet::new(),
            skip_filter: CircularFilter::new(),
            pd: Default::default(),
        }
    }

    fn sync_backlog(&mut self, pair: &PR, update: Channel<OrderUpdate<Bundled<SO, B>, SO>>)
    where
        PR: Copy + Eq + Hash + Display,
        V: Copy + Eq + Hash + Display,
        SO: SpecializedOrder<TOrderId = V>,
        L: HotBacklog<Bundled<SO, B>> + Maker<MC>,
        MC: Clone,
    {
        let is_confirmed = matches!(update, Channel::Ledger(_));
        let (Channel::Ledger(Confirmed(upd))
        | Channel::Mempool(Unconfirmed(upd))
        | Channel::LocalTxSubmit(Predicted(upd))) = update;
        match upd {
            OrderUpdate::Created(new_order) => {
                let ver = SpecializedOrder::get_self_ref(&new_order);
                if !self.skip_filter.contains(&ver) {
                    self.multi_backlog.get_mut(pair).put(new_order)
                }
            }
            OrderUpdate::Eliminated(elim_order) => {
                let elim_order_id = elim_order.get_self_ref();
                if is_confirmed {
                    self.multi_backlog.get_mut(pair).remove(elim_order_id);
                } else {
                    self.multi_backlog.get_mut(pair).soft_evict(elim_order_id);
                }
            }
        }
    }

    fn sync_book(
        &mut self,
        pair: &PR,
        transition: Ior<Either<Baked<CO, V>, Baked<P, V>>, Either<Baked<CO, V>, Baked<P, V>>>,
    ) where
        PR: Copy + Eq + Hash + Display,
        SID: Copy + Eq + Hash + Display + Debug,
        V: Copy + Eq + Hash + Display,
        B: Clone,
        MC: Clone,
        CO: Stable<StableId = SID> + Clone,
        P: Stable<StableId = SID> + Clone,
        IX: StateIndex<EvolvingEntity<CO, P, V, B>>,
        CH: KvStore<SID, EvolvingEntity<CO, P, V, B>>,
        TLB: ExternalTLBEvents<CO, P> + Maker<MC>,
    {
        trace!(target: "executor", "syncing book pair: {}", pair);
        match transition {
            Ior::Left(e) => match e {
                Either::Left(o) => self.multi_book.get_mut(pair).remove_fragment(o.entity),
                Either::Right(p) => self.multi_book.get_mut(pair).remove_pool(p.entity),
            },
            Ior::Both(old, new) => match (old, new) {
                (Either::Left(old), Either::Left(new)) => {
                    self.multi_book.get_mut(pair).remove_fragment(old.entity);
                    self.multi_book.get_mut(pair).add_fragment(new.entity);
                }
                (_, Either::Right(new)) => {
                    self.multi_book.get_mut(pair).update_pool(new.entity);
                }
                _ => unreachable!(),
            },
            Ior::Right(new) => match new {
                Either::Left(new) => self.multi_book.get_mut(pair).add_fragment(new.entity),
                Either::Right(new) => self.multi_book.get_mut(pair).update_pool(new.entity),
            },
        }
    }

    fn cache<T>(&mut self, new_entity_state: Bundled<T, B>) -> Option<Ior<T, T>>
    where
        SID: Copy + Eq + Hash + Display,
        V: Copy + Eq + Hash + Display,
        T: EntitySnapshot<StableId = SID, Version = V> + Clone,
        B: Clone,
        CH: KvStore<SID, Bundled<T, B>>,
    {
        if let Some(Bundled(prev_best_state, _)) = self
            .cache
            .insert(new_entity_state.stable_id(), new_entity_state.clone())
        {
            Some(Ior::Both(prev_best_state, new_entity_state.0))
        } else {
            Some(Ior::Right(new_entity_state.0))
        }
    }

    fn invalidate_versions(&mut self, pair: &PR, versions: HashSet<V>)
    where
        PR: Copy + Eq + Hash + Display,
        SID: Copy + Eq + Hash + Debug + Display,
        V: Copy + Eq + Hash + Display,
        B: Clone + Debug,
        MC: Clone,
        CO: Stable<StableId = SID> + Clone + Display,
        P: Stable<StableId = SID> + Clone,
        IX: StateIndex<EvolvingEntity<CO, P, V, B>>,
        CH: KvStore<SID, EvolvingEntity<CO, P, V, B>>,
        TLB: ExternalTLBEvents<CO, P> + Maker<MC>,
    {
        for ver in versions {
            if let Some(stable_id) = self.index.invalidate_version(ver) {
                trace!("Invalidating snapshot {} of {}", ver, stable_id);
                let maybe_transition = match resolve_source_state(stable_id, &self.index) {
                    None => self
                        .cache
                        .remove(stable_id)
                        .map(|Bundled(elim_state, _)| Ior::Left(elim_state)),
                    Some(latest_state) => self.cache(latest_state),
                };
                if let Some(tr) = maybe_transition {
                    trace!("Resulting transition is {}", tr);
                    self.sync_book(pair, tr);
                }
            }
        }
    }

    fn update_state<T>(&mut self, update: Channel<StateUpdate<Bundled<T, B>>>) -> Option<Ior<T, T>>
    where
        SID: Copy + Eq + Hash + Display,
        V: Copy + Eq + Hash + Display,
        T: EntitySnapshot<StableId = SID, Version = V> + Clone,
        B: Clone,
        IX: StateIndex<Bundled<T, B>>,
        CH: KvStore<SID, Bundled<T, B>>,
    {
        let from_ledger = matches!(update, Channel::Ledger(_));
        let from_mempool = matches!(update, Channel::Mempool(_));
        let (Channel::Ledger(Confirmed(upd))
        | Channel::Mempool(Unconfirmed(upd))
        | Channel::LocalTxSubmit(Predicted(upd))) = update;
        if let StateUpdate::TransitionRollback(Ior::Both(rolled_back_state, _)) = &upd {
            trace!(
                "State {} was eliminated in result of rollback.",
                rolled_back_state.stable_id()
            );
            self.index.invalidate_version(rolled_back_state.version());
        }
        match upd {
            StateUpdate::Transition(Ior::Right(new_state))
            | StateUpdate::Transition(Ior::Both(_, new_state))
            | StateUpdate::TransitionRollback(Ior::Right(new_state))
            | StateUpdate::TransitionRollback(Ior::Both(_, new_state)) => {
                if self.skip_filter.contains(&new_state.version()) {
                    trace!("State transition of {} is skipped", new_state.stable_id());
                    return None;
                }
                let id = new_state.stable_id();
                if from_ledger {
                    trace!("Observing new confirmed state {}", id);
                    self.index.put_confirmed(Confirmed(new_state));
                } else if from_mempool {
                    trace!("Observing new unconfirmed state {}", id);
                    self.index.put_unconfirmed(Unconfirmed(new_state));
                } else {
                    trace!("Observing new predicted state {}", id);
                    self.index.put_predicted(Predicted(new_state));
                }
                match resolve_source_state(id, &self.index) {
                    Some(latest_state) => self.cache(latest_state),
                    None => unreachable!(),
                }
            }
            StateUpdate::Transition(Ior::Left(st)) | StateUpdate::TransitionRollback(Ior::Left(st)) => {
                self.index.eliminate(st.stable_id());
                Some(Ior::Left(st.0))
            }
        }
    }

    fn on_execution_effects_success(
        &mut self,
        ExecutionEffectsByPair {
            pair,
            tx_hash,
            consumed_versions,
            pending_effects,
        }: ExecutionEffectsByPair<PR, TH, CO, SO, P, V, B>,
    ) where
        SID: Eq + Hash + Copy + Display + Debug,
        V: Eq + Hash + Copy + Display,
        B: Clone + Debug,
        MC: Clone,
        PR: Eq + Hash + Copy + Display,
        SO: SpecializedOrder<TOrderId = V>,
        CO: Stable<StableId = SID> + Copy + Debug,
        P: Stable<StableId = SID> + Copy,
        CH: KvStore<SID, EvolvingEntity<CO, P, V, B>>,
        TH: Display,
        IX: StateIndex<EvolvingEntity<CO, P, V, B>>,
        TLB: ExternalTLBEvents<CO, P> + TLBFeedback<CO, P> + Maker<MC>,
        L: HotBacklog<Bundled<SO, B>> + Maker<MC>,
    {
        trace!("TX {} succeeded", tx_hash);
        match pending_effects {
            ExecutionEffects::FromLiquidityBook(mut pending_effects) => {
                while let Some(effect) = pending_effects.pop() {
                    match effect {
                        ExecutionEff::Updated(elim, upd) => {
                            self.on_entity_processed(elim.version());
                            self.update_state(Channel::local_tx_submit(StateUpdate::Transition(Ior::Both(
                                elim, upd,
                            ))));
                        }
                        ExecutionEff::Eliminated(elim) => {
                            self.on_entity_processed(elim.version());
                            self.update_state(Channel::local_tx_submit(StateUpdate::Transition(Ior::Left(
                                elim,
                            ))));
                        }
                    }
                }
                self.multi_book.get_mut(&pair).on_recipe_succeeded();
            }
            ExecutionEffects::FromBacklog(new_pool, consumed_ord) => {
                self.on_entity_processed(consumed_ord.get_self_ref());
                self.update_state(Channel::local_tx_submit(StateUpdate::Transition(Ior::Right(
                    new_pool.map(Either::Right),
                ))));
            }
        }
    }

    fn on_execution_effects_failure(
        &mut self,
        err: E,
        ExecutionEffectsByPair {
            pair,
            tx_hash,
            consumed_versions,
            pending_effects,
        }: ExecutionEffectsByPair<PR, TH, CO, SO, P, V, B>,
    ) where
        SID: Eq + Hash + Copy + Display + Debug,
        V: Eq + Hash + Copy + Display,
        B: Clone + Debug,
        MC: Clone,
        PR: Eq + Hash + Copy + Display,
        SO: SpecializedOrder<TOrderId = V>,
        CO: Stable<StableId = SID> + Copy + Display,
        P: Stable<StableId = SID> + Copy,
        CH: KvStore<SID, EvolvingEntity<CO, P, V, B>>,
        TH: Display,
        IX: StateIndex<EvolvingEntity<CO, P, V, B>>,
        TLB: ExternalTLBEvents<CO, P> + TLBFeedback<CO, P> + Maker<MC>,
        L: HotBacklog<Bundled<SO, B>> + Maker<MC>,
        E: TryInto<HashSet<V>> + Unpin + Debug + Display,
    {
        warn!("TX {} failed {:?}", tx_hash, err);
        if let Ok(missing_bearers) = err.try_into() {
            match pending_effects {
                ExecutionEffects::FromLiquidityBook(_) => {
                    self.multi_book
                        .get_mut(&pair)
                        .on_recipe_failed(StashingOption::Unstash);
                }
                ExecutionEffects::FromBacklog(_, order) => {
                    let order_ref = order.get_self_ref();
                    if missing_bearers.contains(&order_ref) {
                        self.multi_backlog.get_mut(&pair).soft_evict(order_ref);
                    } else {
                        self.multi_backlog.get_mut(&pair).put(order);
                    }
                }
            }
            // Defensive programming against node sending error for an irrelevant TX.
            let has_relevant_bearers = missing_bearers.intersection(&consumed_versions).next().is_some();
            if has_relevant_bearers {
                trace!("Going to process missing bearers");
                self.invalidate_versions(&pair, missing_bearers.clone());
            }
        } else {
            warn!("Unknown Tx submission error!");
            match pending_effects {
                ExecutionEffects::FromLiquidityBook(_) => {
                    self.multi_book
                        .get_mut(&pair)
                        .on_recipe_failed(StashingOption::Unstash);
                }
                ExecutionEffects::FromBacklog(_, order) => {
                    self.multi_backlog.get_mut(&pair).put(order);
                }
            }
        }
    }

    fn on_funding_effects_success(&mut self, mut effects: Vec<FundingEvent<B>>)
    where
        B: Eq + Ord,
    {
        while let Some(effect) = effects.pop() {
            match effect {
                FundingEvent::Consumed(_) => {}
                FundingEvent::Produced(funding) => {
                    self.funding_pool.insert(funding);
                }
            }
        }
    }

    fn on_funding_effects_failure(&mut self, err: E, mut effects: Vec<FundingEvent<B>>)
    where
        B: Has<V> + Eq + Ord + Clone,
        V: Eq + Hash,
        E: TryInto<HashSet<V>> + Unpin + Debug + Display,
    {
        let missing_bearers = err.try_into().unwrap_or(HashSet::new());
        while let Some(effect) = effects.pop() {
            match effect {
                FundingEvent::Produced(_) => {}
                FundingEvent::Consumed(funding) => {
                    if missing_bearers.contains(&funding.select::<V>()) {
                        self.funding_pool.insert(funding);
                    }
                }
            }
        }
    }

    fn on_entity_processed(&mut self, ver: V)
    where
        V: Copy + Eq + Hash + Display,
    {
        trace!("Saving {} to skip filter", ver);
        self.skip_filter.add(ver);
    }

    fn on_pair_event(&mut self, pair: PR, event: Event<CO, SO, P, B, V>)
    where
        SID: Eq + Hash + Copy + Display + Debug,
        V: Eq + Hash + Copy + Display,
        B: Clone + Debug,
        MC: Clone,
        PR: Eq + Hash + Copy + Display,
        SO: SpecializedOrder<TOrderId = V>,
        CO: Stable<StableId = SID> + Copy + Debug,
        P: Stable<StableId = SID> + Copy,
        CH: KvStore<SID, EvolvingEntity<CO, P, V, B>>,
        IX: StateIndex<EvolvingEntity<CO, P, V, B>>,
        TLB: ExternalTLBEvents<CO, P> + Maker<MC>,
        L: HotBacklog<Bundled<SO, B>> + Maker<MC>,
    {
        match event {
            Either::Left(evolving_entity) => {
                if let Some(upd) = self.update_state(evolving_entity) {
                    self.sync_book(&pair, upd)
                }
            }
            Either::Right(atomic_entity) => self.sync_backlog(&pair, atomic_entity),
        }
        self.focus_set.push_back(pair);
    }

    fn on_funding_event(&mut self, event: FundingEvent<B>)
    where
        B: Eq + Ord,
    {
        match event {
            FundingEvent::Consumed(funding_bearer) => {
                self.funding_pool.remove(&funding_bearer);
            }
            FundingEvent::Produced(funding_bearer) => {
                self.funding_pool.insert(funding_bearer);
            }
        }
    }
}

impl<S, F, PR, SID, V, CO, SO, P, B, TC, TX, TH, U, C, MC, IX, CH, TLB, L, RIR, SIR, PRV, E> Stream
    for Executor<S, F, PR, SID, V, CO, SO, P, B, TC, TX, TH, C, MC, IX, CH, TLB, L, RIR, SIR, PRV, E>
where
    S: Stream<Item = (PR, Event<CO, SO, P, B, V>)> + Unpin,
    F: Stream<Item = FundingEvent<B>> + Unpin,
    PR: Copy + Eq + Ord + Hash + Display + Unpin,
    SID: Copy + Eq + Hash + Debug + Display + Unpin,
    V: Copy + Eq + Hash + Display + Unpin,
    P: Stable<StableId = SID> + Copy + Debug + Unpin + Display,
    CO: Stable<StableId = SID> + MarketTaker<U = U> + Copy + Debug + Unpin + Display,
    SO: SpecializedOrder<TPoolId = SID, TOrderId = V> + Unpin,
    B: Has<V> + Eq + Ord + Clone + Debug + Unpin,
    TC: Unpin,
    TX: CanonicalHash<Hash = TH> + Unpin,
    TH: Display + Unpin,
    C: Clone + Unpin,
    MC: Clone + Unpin,
    IX: StateIndex<EvolvingEntity<CO, P, V, B>> + Unpin,
    CH: KvStore<SID, EvolvingEntity<CO, P, V, B>> + Unpin,
    TLB: TemporalLiquidityBook<CO, P> + ExternalTLBEvents<CO, P> + TLBFeedback<CO, P> + Maker<MC> + Unpin,
    L: HotBacklog<Bundled<SO, B>> + Maker<MC> + Unpin,
    RIR: RecipeInterpreter<CO, P, C, V, B, TC> + Unpin,
    SIR: SpecializedInterpreter<P, SO, V, TC, B, C> + Unpin,
    PRV: TxProver<TC, TX> + Unpin,
    E: TryInto<HashSet<V>> + Clone + Unpin + Debug + Display,
{
    type Item = TX;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context) -> Poll<Option<Self::Item>> {
        loop {
            // Wait for the feedback from the last pending job.
            if !self.pending_effects.is_empty() {
                if let Poll::Ready(Some(result)) = Stream::poll_next(Pin::new(&mut self.feedback), cx) {
                    match result {
                        Ok(_) => {
                            while let Some(effect) = self.pending_effects.pop() {
                                match effect {
                                    Effects::Pair(execution_effects) => {
                                        self.on_execution_effects_success(execution_effects)
                                    }
                                    Effects::Funding(funding_effects) => {
                                        self.on_funding_effects_success(funding_effects)
                                    }
                                }
                            }
                        }
                        Err(err) => {
                            while let Some(effect) = self.pending_effects.pop() {
                                match effect {
                                    Effects::Pair(execution_effects) => {
                                        self.on_execution_effects_failure(err.clone(), execution_effects)
                                    }
                                    Effects::Funding(funding_effects) => {
                                        self.on_funding_effects_failure(err.clone(), funding_effects)
                                    }
                                }
                            }
                        }
                    }
                }
            }
            // Process all upstream events before matchmaking.
            if let Poll::Ready(Some((pair, event))) = Stream::poll_next(Pin::new(&mut self.upstream), cx) {
                self.on_pair_event(pair, event);
                continue;
            }
            // Process all funding events before matchmaking.
            if let Poll::Ready(Some(funding_event)) =
                Stream::poll_next(Pin::new(&mut self.funding_events), cx)
            {
                self.on_funding_event(funding_event);
                continue;
            }
            // Finally attempt to matchmake.
            while let Some(focus_pair) = self.focus_set.pop_front() {
                // Try TLB:
                if let Some(recipe) = self.multi_book.get_mut(&focus_pair).attempt() {
                    let (linked_recipe, consumed_versions) = ExecutionRecipe::link(recipe, |id| {
                        self.cache
                            .get(id)
                            .map(|Bundled(t, bearer)| (t.either(|b| b.version, |b| b.version), bearer))
                    })
                    .expect("State is inconsistent");
                    let ctx = self.context.clone();
                    if let Some(funding) = self.funding_pool.pop_first() {
                        let ExecutionResult {
                            txc,
                            matchmaking_effects,
                            funding_io,
                        } = self.trade_interpreter.run(linked_recipe, funding, ctx);
                        let tx = self.prover.prove(txc);
                        let tx_hash = tx.canonical_hash();
                        self.pending_effects.push(Effects::Pair(ExecutionEffectsByPair {
                            pair: focus_pair,
                            tx_hash,
                            consumed_versions,
                            pending_effects: ExecutionEffects::FromLiquidityBook(matchmaking_effects),
                        }));
                        let (maybe_unused_funding, funding_effects) = funding_io.into_effects();
                        if let Some(unused_funding) = maybe_unused_funding {
                            self.funding_pool.insert(unused_funding);
                        }
                        self.pending_effects.push(Effects::Funding(funding_effects));
                        // Return pair to focus set to make sure corresponding TLB will be exhausted.
                        self.focus_set.push_back(focus_pair);
                        return Poll::Ready(Some(tx));
                    } else {
                        warn!("Cannot matchmake without funding box");
                        self.multi_book
                            .get_mut(&focus_pair)
                            .on_recipe_failed(StashingOption::Unstash);
                    }
                }
                // Try Backlog:
                if let Some(next_order) = self.multi_backlog.get_mut(&focus_pair).try_pop() {
                    if let Some(Bundled(Either::Right(pool), pool_bearer)) =
                        self.cache.get(next_order.0.get_pool_ref())
                    {
                        let ctx = self.context.clone();
                        if let Some((txc, updated_pool, consumed_ord)) =
                            self.spec_interpreter
                                .try_run(Bundled(pool.entity, pool_bearer), next_order, ctx)
                        {
                            let tx = self.prover.prove(txc);
                            let tx_hash = tx.canonical_hash();
                            let consumed_versions =
                                HashSet::from_iter(vec![pool.version, consumed_ord.get_self_ref()]);
                            self.pending_effects.push(Effects::Pair(ExecutionEffectsByPair {
                                pair: focus_pair,
                                tx_hash,
                                consumed_versions,
                                pending_effects: ExecutionEffects::FromBacklog(updated_pool, consumed_ord),
                            }));
                            // Return pair to focus set to make sure corresponding TLB will be exhausted.
                            self.focus_set.push_back(focus_pair);
                            return Poll::Ready(Some(tx));
                        }
                    }
                }
            }
            return Poll::Pending;
        }
    }
}

impl<S, F, PR, ST, V, CO, SO, P, B, TC, TX, TH, U, C, MC, IX, CH, TLB, L, RIR, SIR, PRV, E> FusedStream
    for Executor<S, F, PR, ST, V, CO, SO, P, B, TC, TX, TH, C, MC, IX, CH, TLB, L, RIR, SIR, PRV, E>
where
    S: Stream<Item = (PR, Event<CO, SO, P, B, V>)> + Unpin,
    F: Stream<Item = FundingEvent<B>> + Unpin,
    PR: Copy + Eq + Ord + Hash + Display + Unpin,
    ST: Copy + Eq + Hash + Debug + Display + Unpin,
    V: Copy + Eq + Hash + Display + Unpin,
    P: Stable<StableId = ST> + Copy + Debug + Unpin + Display,
    CO: Stable<StableId = ST> + MarketTaker<U = U> + Copy + Debug + Unpin + Display,
    SO: SpecializedOrder<TPoolId = ST, TOrderId = V> + Unpin,
    B: Has<V> + Eq + Ord + Clone + Debug + Unpin,
    TC: Unpin,
    TX: CanonicalHash<Hash = TH> + Unpin,
    TH: Display + Unpin,
    C: Clone + Unpin,
    MC: Clone + Unpin,
    IX: StateIndex<EvolvingEntity<CO, P, V, B>> + Unpin,
    CH: KvStore<ST, EvolvingEntity<CO, P, V, B>> + Unpin,
    TLB: TemporalLiquidityBook<CO, P> + ExternalTLBEvents<CO, P> + TLBFeedback<CO, P> + Maker<MC> + Unpin,
    L: HotBacklog<Bundled<SO, B>> + Maker<MC> + Unpin,
    RIR: RecipeInterpreter<CO, P, C, V, B, TC> + Unpin,
    SIR: SpecializedInterpreter<P, SO, V, TC, B, C> + Unpin,
    PRV: TxProver<TC, TX> + Unpin,
    E: TryInto<HashSet<V>> + Clone + Unpin + Debug + Display,
{
    fn is_terminated(&self) -> bool {
        false
    }
}
