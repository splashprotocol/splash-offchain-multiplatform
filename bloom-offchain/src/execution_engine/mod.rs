use std::collections::HashSet;
use std::fmt::{Debug, Display};
use std::hash::Hash;
use std::marker::PhantomData;
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
use spectrum_offchain::data::{Baked, EntitySnapshot, Stable};
use spectrum_offchain::health_alert::{HealthAlertClient, SlackHealthAlert};
use spectrum_offchain::maker::Maker;
use spectrum_offchain::network::Network;
use spectrum_offchain::tx_hash::CanonicalHash;
use spectrum_offchain::tx_prover::TxProver;

use crate::execution_engine::backlog::SpecializedInterpreter;
use crate::execution_engine::bundled::Bundled;
use crate::execution_engine::execution_effect::ExecutionEff;
use crate::execution_engine::focus_set::FocusSet;
use crate::execution_engine::liquidity_book::fragment::{Fragment, OrderState};
use crate::execution_engine::liquidity_book::recipe::{
    ExecutionRecipe, LinkedExecutionRecipe, LinkedFill, LinkedSwap, LinkedTerminalInstruction,
    TerminalInstruction,
};
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

pub enum PendingEffects<CompOrd, SpecOrd, Pool, Ver, Bearer> {
    FromLiquidityBook(
        Vec<ExecutionEff<EvolvingEntity<CompOrd, Pool, Ver, Bearer>, Bundled<Baked<CompOrd, Ver>, Bearer>>>,
    ),
    FromBacklog(Bundled<Baked<Pool, Ver>, Bearer>, Bundled<SpecOrd, Bearer>),
}

/// Instantiate execution stream partition.
/// Each partition serves total_pairs/num_partitions pairs.
pub fn execution_part_stream<
    'a,
    Upstream,
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
    book: MultiPair<Pair, Book, Ctx>,
    backlog: MultiPair<Pair, Backlog, Ctx>,
    context: Ctx,
    rec_interpreter: RecInterpreter,
    spec_interpreter: SpecInterpreter,
    prover: Prover,
    upstream: Upstream,
    network: Net,
    mut tip_reached_signal: broadcast::Receiver<bool>,
    alert_client: HealthAlertClient,
) -> impl Stream<Item = ()> + 'a
where
    Upstream: Stream<Item = (Pair, Event<CompOrd, SpecOrd, Pool, Bearer, Ver>)> + Unpin + 'a,
    Pair: Copy + Eq + Ord + Hash + Display + Unpin + 'a,
    StableId: Copy + Eq + Hash + Debug + Display + Unpin + 'a,
    Ver: Copy + Eq + Hash + Display + Unpin + 'a,
    Pool: Stable<StableId = StableId> + Copy + Debug + Unpin + Display + 'a,
    CompOrd: Stable<StableId = StableId>
        + Fragment<U = ExUnits>
        + OrderState
        + Copy
        + Debug
        + Unpin
        + Display
        + 'a,
    SpecOrd: SpecializedOrder<TPoolId = StableId, TOrderId = Ver> + Debug + Unpin + 'a,
    Bearer: Clone + Unpin + Debug + 'a,
    TxCandidate: Unpin + 'a,
    Tx: CanonicalHash<Hash = TxHash> + Unpin + 'a,
    TxHash: Display + Unpin + 'a,
    Ctx: Clone + Unpin + 'a,
    Index: StateIndex<EvolvingEntity<CompOrd, Pool, Ver, Bearer>> + Unpin + 'a,
    Cache: KvStore<StableId, EvolvingEntity<CompOrd, Pool, Ver, Bearer>> + Unpin + 'a,
    Book: TemporalLiquidityBook<CompOrd, Pool>
        + ExternalTLBEvents<CompOrd, Pool>
        + TLBFeedback<CompOrd, Pool>
        + Maker<Ctx>
        + Unpin
        + 'a,
    Backlog: HotBacklog<Bundled<SpecOrd, Bearer>> + Maker<Ctx> + Unpin + 'a,
    RecInterpreter: RecipeInterpreter<CompOrd, Pool, Ctx, Ver, Bearer, TxCandidate> + Unpin + 'a,
    SpecInterpreter: SpecializedInterpreter<Pool, SpecOrd, Ver, TxCandidate, Bearer, Ctx> + Unpin + 'a,
    Prover: TxProver<TxCandidate, Tx> + Unpin + 'a,
    Net: Network<Tx, Err> + Clone + 'a,
    Err: TryInto<HashSet<Ver>> + Unpin + Debug + Display + 'a,
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
        feedback_in,
        alert_client,
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
    multi_book: MultiPair<Pair, Book, Ctx>,
    /// Separate Backlogs for each pair (for specialized operations such as Deposit/Redeem)
    multi_backlog: MultiPair<Pair, Backlog, Ctx>,
    context: Ctx,
    trade_interpreter: TradeInterpreter,
    spec_interpreter: SpecInterpreter,
    prover: Prover,
    upstream: Upstream,
    /// Feedback channel is used to signal the status of transaction submitted earlier by the executor.
    feedback: mpsc::Receiver<Result<(), Err>>,
    /// Pending effects resulted from execution of a batch trade in a certain [Pair].
    pending_effects: Option<(Pair, TxHash, PendingEffects<CompOrd, SpecOrd, Pool, Ver, Bearer>)>,
    /// Which pair should we process in the first place.
    focus_set: FocusSet<Pair>,
    /// Temporarily memoize entities that came from unconfirmed updates.
    skip_filter: CircularFilter<128, Ver>,
    pd: PhantomData<(StableId, Ver, TxCandidate, Tx, Err)>,
    alert_client: HealthAlertClient,
}

impl<S, PR, SID, V, CO, SO, P, B, TC, TX, TH, C, IX, CH, TLB, L, RIR, SIR, PRV, E>
    Executor<S, PR, SID, V, CO, SO, P, B, TC, TX, TH, C, IX, CH, TLB, L, RIR, SIR, PRV, E>
{
    fn new(
        index: IX,
        cache: CH,
        multi_book: MultiPair<PR, TLB, C>,
        multi_backlog: MultiPair<PR, L, C>,
        context: C,
        trade_interpreter: RIR,
        spec_interpreter: SIR,
        prover: PRV,
        upstream: S,
        feedback: mpsc::Receiver<Result<(), E>>,
        alert_client: HealthAlertClient,
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
            feedback,
            pending_effects: None,
            focus_set: FocusSet::new(),
            skip_filter: CircularFilter::new(),
            pd: Default::default(),
            alert_client,
        }
    }

    fn sync_backlog(&mut self, pair: &PR, update: Channel<OrderUpdate<Bundled<SO, B>, SO>>)
    where
        PR: Copy + Eq + Hash + Display,
        V: Copy + Eq + Hash + Display,
        SO: SpecializedOrder<TOrderId = V>,
        L: HotBacklog<Bundled<SO, B>> + Maker<C>,
        C: Clone,
    {
        let is_confirmed = matches!(update, Channel::Ledger(_));
        let (Channel::Ledger(Confirmed(upd))
        | Channel::Mempool(Unconfirmed(upd))
        | Channel::TxSubmit(Predicted(upd))) = update;
        match upd {
            OrderUpdate::Created(new_order) => {
                let ver = SpecializedOrder::get_self_ref(&new_order);
                let seen_recently = self.skip_filter.contains(&ver);
                if seen_recently {
                    if is_confirmed {
                        self.skip_filter.remove(&ver);
                    }
                } else {
                    if is_confirmed {
                        self.multi_backlog.get_mut(pair).put(new_order)
                    } else {
                        self.multi_backlog.get_mut(pair).put(new_order)
                    }
                    self.skip_filter.add(ver);
                }
            }
            OrderUpdate::Eliminated(elim_order) => {
                self.multi_backlog.get_mut(pair).remove(elim_order.get_self_ref())
            }
        }
    }

    fn sync_book(
        &mut self,
        pair: &PR,
        transition: Ior<Either<Baked<CO, V>, Baked<P, V>>, Either<Baked<CO, V>, Baked<P, V>>>,
    ) where
        PR: Copy + Eq + Hash + Display,
        SID: Copy + Eq + Hash + Debug + Display,
        V: Copy + Eq + Hash + Display,
        B: Clone + Debug,
        C: Clone,
        CO: Stable<StableId = SID> + Clone + Debug,
        P: Stable<StableId = SID> + Clone + Debug,
        IX: StateIndex<EvolvingEntity<CO, P, V, B>>,
        CH: KvStore<SID, EvolvingEntity<CO, P, V, B>>,
        TLB: ExternalTLBEvents<CO, P> + Maker<C>,
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
        C: Clone,
        CO: Stable<StableId = SID> + Clone + Debug + Display,
        P: Stable<StableId = SID> + Clone + Debug,
        IX: StateIndex<EvolvingEntity<CO, P, V, B>>,
        CH: KvStore<SID, EvolvingEntity<CO, P, V, B>>,
        TLB: ExternalTLBEvents<CO, P> + Maker<C>,
    {
        for ver in versions {
            if let Some(stable_id) = self.index.invalidate_version(ver) {
                trace!("Invalidating snapshot of {}", stable_id);
                let maybe_transition = match resolve_source_state(stable_id, &self.index) {
                    None => self
                        .cache
                        .remove(stable_id)
                        .map(|Bundled(elim_state, _)| Ior::Left(elim_state)),
                    Some(latest_state) => self.cache(latest_state),
                };
                if let Some(tr) = maybe_transition {
                    trace!("Resulting transition is {}", tr);
                    self.sync_book(pair, tr)
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
        | Channel::TxSubmit(Predicted(upd))) = update;
        match upd {
            StateUpdate::Transition(Ior::Right(new_state))
            | StateUpdate::Transition(Ior::Both(_, new_state))
            | StateUpdate::TransitionRollback(Ior::Right(new_state))
            | StateUpdate::TransitionRollback(Ior::Both(_, new_state)) => {
                let id = new_state.stable_id();
                let ver = new_state.version();
                if from_ledger {
                    trace!("Observing new confirmed state {}", id);
                    self.index.put_confirmed(Confirmed(new_state));
                } else {
                    if from_mempool {
                        trace!("Observing new unconfirmed state {}", id);
                        self.index.put_unconfirmed(Unconfirmed(new_state));
                    } else {
                        trace!("Observing new predicted state {}", id);
                        self.index.put_predicted(Predicted(new_state));
                    }
                }
                let seen_recently = self.skip_filter.contains(&ver);
                let state_exists = self.index.exists(&ver);
                if state_exists || seen_recently {
                    if from_ledger {
                        self.skip_filter.remove(&ver);
                    }
                    // No TLB update is needed.
                    return None;
                } else {
                    self.skip_filter.add(ver);
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

    fn link_recipe(&self, recipe: ExecutionRecipe<CO, P>) -> LinkedExecutionRecipe<CO, P, B>
    where
        SID: Copy + Eq + Hash + Debug + Display,
        V: Copy + Eq + Hash + Display,
        CO: Stable<StableId = SID> + Debug,
        P: Stable<StableId = SID> + Debug,
        CH: KvStore<SID, EvolvingEntity<CO, P, V, B>>,
    {
        let mut xs = recipe.instructions();
        let mut linked = vec![];
        while let Some(i) = xs.pop() {
            match i {
                TerminalInstruction::Fill(fill) => {
                    let id = fill.target_fr.stable_id();
                    let Bundled(_, bearer) = self.cache.get(id).expect("State is inconsistent");
                    linked.push(LinkedTerminalInstruction::Fill(LinkedFill::from_fill(
                        fill, bearer,
                    )));
                }
                TerminalInstruction::Swap(swap) => {
                    let id = swap.target.stable_id();
                    let Bundled(_, bearer) = self.cache.get(id).expect("State is inconsistent");
                    linked.push(LinkedTerminalInstruction::Swap(LinkedSwap::from_swap(
                        swap, bearer,
                    )));
                }
            }
        }
        LinkedExecutionRecipe(linked)
    }
}

impl<S, PR, SID, V, CO, SO, P, B, TC, TX, TH, U, C, IX, CH, TLB, L, RIR, SIR, PRV, E> Stream
    for Executor<S, PR, SID, V, CO, SO, P, B, TC, TX, TH, C, IX, CH, TLB, L, RIR, SIR, PRV, E>
where
    S: Stream<Item = (PR, Event<CO, SO, P, B, V>)> + Unpin,
    PR: Copy + Eq + Ord + Hash + Display + Unpin,
    SID: Copy + Eq + Hash + Debug + Display + Unpin,
    V: Copy + Eq + Hash + Display + Unpin,
    P: Stable<StableId = SID> + Copy + Debug + Unpin + Display,
    CO: Stable<StableId = SID> + Fragment<U = U> + OrderState + Copy + Debug + Unpin + Display,
    SO: SpecializedOrder<TPoolId = SID, TOrderId = V> + Unpin,
    B: Clone + Debug + Unpin,
    TC: Unpin,
    TX: CanonicalHash<Hash = TH> + Unpin,
    TH: Display + Unpin,
    C: Clone + Unpin,
    IX: StateIndex<EvolvingEntity<CO, P, V, B>> + Unpin,
    CH: KvStore<SID, EvolvingEntity<CO, P, V, B>> + Unpin,
    TLB: TemporalLiquidityBook<CO, P> + ExternalTLBEvents<CO, P> + TLBFeedback<CO, P> + Maker<C> + Unpin,
    L: HotBacklog<Bundled<SO, B>> + Maker<C> + Unpin,
    RIR: RecipeInterpreter<CO, P, C, V, B, TC> + Unpin,
    SIR: SpecializedInterpreter<P, SO, V, TC, B, C> + Unpin,
    PRV: TxProver<TC, TX> + Unpin,
    E: TryInto<HashSet<V>> + Unpin + Debug + Display,
{
    type Item = TX;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context) -> Poll<Option<Self::Item>> {
        loop {
            // Wait for the feedback from the last pending job.
            if let Some((pair, tx_hash, pending_effects)) = self.pending_effects.take() {
                match Stream::poll_next(Pin::new(&mut self.feedback), cx) {
                    Poll::Ready(Some(result)) => match result {
                        Ok(_) => {
                            trace!("TX {} succeeded", tx_hash);
                            match pending_effects {
                                PendingEffects::FromLiquidityBook(mut pending_effects) => {
                                    while let Some(effect) = pending_effects.pop() {
                                        match effect {
                                            ExecutionEff::Updated(upd) => {
                                                self.update_state(Channel::tx_submit(
                                                    StateUpdate::Transition(Ior::Right(upd)),
                                                ));
                                            }
                                            ExecutionEff::Eliminated(elim) => {
                                                self.update_state(Channel::tx_submit(
                                                    StateUpdate::Transition(Ior::Left(
                                                        elim.map(Either::Left),
                                                    )),
                                                ));
                                            }
                                        }
                                    }
                                    self.multi_book.get_mut(&pair).on_recipe_succeeded();
                                }
                                PendingEffects::FromBacklog(new_pool, _) => {
                                    self.update_state(Channel::tx_submit(StateUpdate::Transition(
                                        Ior::Right(new_pool.map(Either::Right)),
                                    )));
                                }
                            }
                        }
                        Err(err) => {
                            //todo: remove
                            let submit_res = self
                                .alert_client
                                .send_alert(format!("Tx submission error: {}", err).as_str())
                                .unwrap_or("Failure".to_string());

                            trace!("Alert submitting result: {}", submit_res);

                            warn!("TX {} failed {:?}", tx_hash, err);
                            if let Ok(missing_bearers) = err.try_into() {
                                match pending_effects {
                                    PendingEffects::FromLiquidityBook(_) => {
                                        self.multi_book
                                            .get_mut(&pair)
                                            .on_recipe_failed(StashingOption::Unstash);
                                    }
                                    PendingEffects::FromBacklog(_, Bundled(order, br)) => {
                                        let order_ref = order.get_self_ref();
                                        if missing_bearers.contains(&order_ref) {
                                            self.multi_backlog.get_mut(&pair).remove(order_ref);
                                        } else {
                                            self.multi_backlog.get_mut(&pair).recharge(Bundled(order, br));
                                        }
                                    }
                                }
                                self.invalidate_versions(&pair, missing_bearers.clone());
                            } else {
                                warn!("Unknown Tx submission error!");
                                match pending_effects {
                                    PendingEffects::FromLiquidityBook(_) => {
                                        self.multi_book
                                            .get_mut(&pair)
                                            .on_recipe_failed(StashingOption::Unstash);
                                    }
                                    PendingEffects::FromBacklog(_, order) => {
                                        self.multi_backlog.get_mut(&pair).recharge(order);
                                    }
                                }
                            }
                        }
                    },
                    _ => {
                        let _ = self.pending_effects.insert((pair, tx_hash, pending_effects));
                        return Poll::Pending;
                    }
                }
            }
            // Prioritize external updates over local work.
            if let Poll::Ready(Some((pair, update))) = Stream::poll_next(Pin::new(&mut self.upstream), cx) {
                match update {
                    Either::Left(evolving_entity) => {
                        if let Some(upd) = self.update_state(evolving_entity) {
                            self.sync_book(&pair, upd)
                        }
                    }
                    Either::Right(atomic_entity) => self.sync_backlog(&pair, atomic_entity),
                }
                self.focus_set.push_back(pair);
                continue;
            }
            // Finally attempt to execute something.
            while let Some(focus_pair) = self.focus_set.pop_front() {
                // Try TLB:
                if let Some(recipe) = self.multi_book.get_mut(&focus_pair).attempt() {
                    let linked_recipe = self.link_recipe(recipe.into());
                    let ctx = self.context.clone();
                    let (txc, effects) = self.trade_interpreter.run(linked_recipe, ctx);
                    let tx = self.prover.prove(txc);
                    let tx_hash = tx.canonical_hash();
                    let _ = self.pending_effects.insert((
                        focus_pair,
                        tx_hash,
                        PendingEffects::FromLiquidityBook(effects),
                    ));
                    // Return pair to focus set to make sure corresponding TLB will be exhausted.
                    self.focus_set.push_back(focus_pair);
                    return Poll::Ready(Some(tx));
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
                            let _ = self.pending_effects.insert((
                                focus_pair,
                                tx_hash,
                                PendingEffects::FromBacklog(updated_pool, consumed_ord),
                            ));
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

impl<S, PR, ST, V, CO, SO, P, B, TC, TX, TH, U, C, IX, CH, TLB, L, RIR, SIR, PRV, E> FusedStream
    for Executor<S, PR, ST, V, CO, SO, P, B, TC, TX, TH, C, IX, CH, TLB, L, RIR, SIR, PRV, E>
where
    S: Stream<Item = (PR, Event<CO, SO, P, B, V>)> + Unpin,
    PR: Copy + Eq + Ord + Hash + Display + Unpin,
    ST: Copy + Eq + Hash + Debug + Display + Unpin,
    V: Copy + Eq + Hash + Display + Unpin,
    P: Stable<StableId = ST> + Copy + Debug + Unpin + Display,
    CO: Stable<StableId = ST> + Fragment<U = U> + OrderState + Copy + Debug + Unpin + Display,
    SO: SpecializedOrder<TPoolId = ST, TOrderId = V> + Unpin,
    B: Clone + Debug + Unpin,
    TC: Unpin,
    TX: CanonicalHash<Hash = TH> + Unpin,
    TH: Display + Unpin,
    C: Clone + Unpin,
    IX: StateIndex<EvolvingEntity<CO, P, V, B>> + Unpin,
    CH: KvStore<ST, EvolvingEntity<CO, P, V, B>> + Unpin,
    TLB: TemporalLiquidityBook<CO, P> + ExternalTLBEvents<CO, P> + TLBFeedback<CO, P> + Maker<C> + Unpin,
    L: HotBacklog<Bundled<SO, B>> + Maker<C> + Unpin,
    RIR: RecipeInterpreter<CO, P, C, V, B, TC> + Unpin,
    SIR: SpecializedInterpreter<P, SO, V, TC, B, C> + Unpin,
    PRV: TxProver<TC, TX> + Unpin,
    E: TryInto<HashSet<V>> + Unpin + Debug + Display,
{
    fn is_terminated(&self) -> bool {
        false
    }
}
