use std::collections::BTreeSet;
use std::fmt::Display;
use std::hash::Hash;
use std::marker::PhantomData;
use std::pin::Pin;
use std::task::{Context, Poll};

use futures::channel::mpsc;
use futures::future::Either;
use futures::stream::FusedStream;
use futures::Stream;
use futures::{SinkExt, StreamExt};
use log::trace;

use spectrum_offchain::combinators::Ior;
use spectrum_offchain::data::unique_entity::{Confirmed, EitherMod, StateUpdate, Unconfirmed};
use spectrum_offchain::data::EntitySnapshot;
use spectrum_offchain::network::Network;
use spectrum_offchain::tx_prover::TxProver;

use crate::execution_engine::bundled::Bundled;
use crate::execution_engine::interpreter::RecipeInterpreter;
use crate::execution_engine::liquidity_book::fragment::{Fragment, OrderState};
use crate::execution_engine::liquidity_book::recipe::{
    ExecutionRecipe, LinkedExecutionRecipe, LinkedFill, LinkedSwap, LinkedTerminalInstruction,
    TerminalInstruction,
};
use crate::execution_engine::liquidity_book::{ExternalTLBEvents, TLBFeedback, TemporalLiquidityBook};
use crate::execution_engine::multi_pair::MultiPair;
use crate::execution_engine::resolver::resolve_source_state;
use crate::execution_engine::storage::cache::StateIndexCache;
use crate::execution_engine::storage::StateIndex;
use crate::maker::Maker;

pub mod batch_exec;
pub mod bundled;
pub mod interpreter;
pub mod liquidity_book;
pub mod multi_pair;
pub mod partial_fill;
pub mod resolver;
pub mod storage;
pub mod types;

pub type Event<O, P, B> = EitherMod<StateUpdate<Bundled<Either<O, P>, B>>>;

/// Instantiate execution stream partition.
/// Each partition serves total_pairs/num_partitions pairs.
pub fn execution_part_stream<
    'a,
    Upstream,
    PairId,
    StableId,
    Ver,
    Order,
    Pool,
    Bearer,
    Txc,
    Tx,
    Ctx,
    Index,
    Cache,
    Book,
    Interpreter,
    Prover,
    Net,
    Err,
>(
    index: Index,
    cache: Cache,
    book: MultiPair<PairId, Book, Ctx>,
    context: Ctx,
    interpreter: Interpreter,
    prover: Prover,
    upstream: Upstream,
    network: Net,
) -> impl Stream<Item = ()> + 'a
where
    Upstream: Stream<Item = (PairId, Event<Order, Pool, Bearer>)> + Unpin + 'a,
    PairId: Copy + Eq + Ord + Hash + Display + Unpin + 'a,
    StableId: Copy + Eq + Hash + Display + Unpin + 'a,
    Ver: Copy + Eq + Hash + Display + Unpin + 'a,
    Pool: EntitySnapshot<StableId = StableId, Version = Ver> + Copy + Unpin + 'a,
    Order: EntitySnapshot<StableId = StableId, Version = Ver> + Fragment + OrderState + Copy + Unpin + 'a,
    Bearer: Clone + Unpin + 'a,
    Txc: Unpin + 'a,
    Tx: Unpin + 'a,
    Ctx: Clone + Unpin + 'a,
    Index: StateIndex<Bundled<Either<Order, Pool>, Bearer>> + Unpin + 'a,
    Cache: StateIndexCache<StableId, Bundled<Either<Order, Pool>, Bearer>> + Unpin + 'a,
    Book: TemporalLiquidityBook<Order, Pool>
        + ExternalTLBEvents<Order, Pool>
        + TLBFeedback<Order, Pool>
        + Maker<Ctx>
        + Unpin
        + 'a,
    Interpreter: RecipeInterpreter<Order, Pool, Ctx, Bearer, Txc> + Unpin + 'a,
    Prover: TxProver<Txc, Tx> + Unpin + 'a,
    Net: Network<Tx, Err> + Clone + 'a,
    Err: Unpin + 'a,
{
    let (feedback_out, feedback_in) = mpsc::channel(100);
    let executor = Executor::new(
        index,
        cache,
        book,
        context,
        interpreter,
        prover,
        upstream,
        feedback_in,
    );
    executor.then(move |tx| {
        let mut network = network.clone();
        let mut feedback = feedback_out.clone();
        async move {
            let result = network.submit_tx(tx).await;
            feedback.send(result).await.expect("Filed to propagate feedback.");
        }
    })
}

pub struct Executor<S, PairId, StableId, Ver, O, P, Bearer, Txc, Tx, C, Index, Cache, Book, Ir, Prover, Err> {
    index: Index,
    cache: Cache,
    /// Separate TLBs for each pair.
    multi_book: MultiPair<PairId, Book, C>,
    context: C,
    interpreter: Ir,
    prover: Prover,
    upstream: S,
    /// Feedback channel is used to deliver the status of transaction produced by the executor back to it.
    feedback: mpsc::Receiver<Result<(), Err>>,
    /// Pending effects resulted from execution of a batch trade in a certain [PairId].
    pending_effects: Option<(PairId, Vec<(Either<O, P>, Bearer)>)>,
    /// Which pair should we process in the first place.
    focus_set: BTreeSet<PairId>,
    pd1: PhantomData<StableId>,
    pd2: PhantomData<Ver>,
    pd3: PhantomData<Txc>,
    pd4: PhantomData<Tx>,
    pd5: PhantomData<Err>,
}

impl<S, PairId, StableId, Ver, O, P, B, Txc, Tx, Ctx, Index, Cache, Book, Ir, Prover, Err>
    Executor<S, PairId, StableId, Ver, O, P, B, Txc, Tx, Ctx, Index, Cache, Book, Ir, Prover, Err>
{
    fn new(
        index: Index,
        cache: Cache,
        multi_book: MultiPair<PairId, Book, Ctx>,
        context: Ctx,
        interpreter: Ir,
        prover: Prover,
        upstream: S,
        feedback: mpsc::Receiver<Result<(), Err>>,
    ) -> Self {
        Self {
            index,
            cache,
            multi_book,
            context,
            interpreter,
            prover,
            upstream,
            feedback,
            pending_effects: None,
            focus_set: Default::default(),
            pd1: Default::default(),
            pd2: Default::default(),
            pd3: Default::default(),
            pd4: Default::default(),
            pd5: Default::default(),
        }
    }

    fn sync_book(&mut self, pair: PairId, update: EitherMod<StateUpdate<Bundled<Either<O, P>, B>>>)
    where
        PairId: Copy + Eq + Hash + Display,
        StableId: Copy + Eq + Hash + Display,
        Ver: Copy + Eq + Hash + Display,
        B: Clone,
        Ctx: Clone,
        O: EntitySnapshot<StableId = StableId, Version = Ver> + Clone,
        P: EntitySnapshot<StableId = StableId, Version = Ver> + Clone,
        Index: StateIndex<Bundled<Either<O, P>, B>>,
        Cache: StateIndexCache<StableId, Bundled<Either<O, P>, B>>,
        Book: ExternalTLBEvents<O, P> + Maker<Ctx>,
    {
        match self.update_state(update) {
            None => {}
            Some(Ior::Left(e)) => match e {
                Either::Left(o) => self.multi_book.get_mut(&pair).remove_fragment(o),
                Either::Right(p) => self.multi_book.get_mut(&pair).remove_pool(p),
            },
            Some(Ior::Both(old, new)) => match (old, new) {
                (Either::Left(old), Either::Left(new)) => {
                    self.multi_book.get_mut(&pair).remove_fragment(old);
                    self.multi_book.get_mut(&pair).add_fragment(new);
                }
                (_, Either::Right(new)) => {
                    self.multi_book.get_mut(&pair).update_pool(new);
                }
                _ => unreachable!(),
            },
            Some(Ior::Right(new)) => match new {
                Either::Left(new) => self.multi_book.get_mut(&pair).add_fragment(new),
                Either::Right(new) => self.multi_book.get_mut(&pair).update_pool(new),
            },
        }
    }

    fn update_state<T>(&mut self, update: EitherMod<StateUpdate<Bundled<T, B>>>) -> Option<Ior<T, T>>
    where
        StableId: Copy + Eq + Hash + Display,
        Ver: Copy + Eq + Hash + Display,
        T: EntitySnapshot<StableId = StableId> + Clone,
        B: Clone,
        Index: StateIndex<Bundled<T, B>>,
        Cache: StateIndexCache<StableId, Bundled<T, B>>,
    {
        let is_confirmed = matches!(update, EitherMod::Confirmed(_));
        let (EitherMod::Confirmed(Confirmed(upd)) | EitherMod::Unconfirmed(Unconfirmed(upd))) = update;
        match upd {
            StateUpdate::Transition(Ior::Right(new_state))
            | StateUpdate::Transition(Ior::Both(_, new_state))
            | StateUpdate::TransitionRollback(Ior::Right(new_state))
            | StateUpdate::TransitionRollback(Ior::Both(_, new_state)) => {
                let id = new_state.stable_id();
                if is_confirmed {
                    trace!("Observing new confirmed state {}", id);
                    self.index.put_confirmed(Confirmed(new_state));
                } else {
                    trace!("Observing new unconfirmed state {}", id);
                    self.index.put_unconfirmed(Unconfirmed(new_state));
                }
                match resolve_source_state(id, &self.index) {
                    Some(latest_state) => {
                        if let Some(Bundled(prev_best_state, _)) = self.cache.insert(latest_state.clone()) {
                            Some(Ior::Both(prev_best_state, latest_state.0))
                        } else {
                            Some(Ior::Right(latest_state.0))
                        }
                    }
                    None => unreachable!(),
                }
            }
            StateUpdate::Transition(Ior::Left(st)) => {
                self.index.eliminate(st.version(), st.stable_id());
                Some(Ior::Left(st.0))
            }
            StateUpdate::TransitionRollback(Ior::Left(st)) => {
                let id = st.stable_id();
                trace!("Rolling back state {}", id);
                self.index.invalidate(st.version(), id);
                Some(Ior::Left(st.0))
            }
        }
    }

    fn link_recipe(&self, ExecutionRecipe(mut xs): ExecutionRecipe<O, P>) -> LinkedExecutionRecipe<O, P, B>
    where
        StableId: Copy + Eq + Hash + Display,
        Ver: Copy + Eq + Hash + Display,
        O: EntitySnapshot<StableId = StableId, Version = Ver>,
        P: EntitySnapshot<StableId = StableId, Version = Ver>,
        Cache: StateIndexCache<StableId, Bundled<Either<O, P>, B>>,
    {
        let mut linked = vec![];
        while let Some(i) = xs.pop() {
            match i {
                TerminalInstruction::Fill(fill) => {
                    let id = fill.target.stable_id();
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

impl<S, PairId, StableId, Ver, O, P, B, Txc, Tx, C, Index, Cache, Book, Ir, Prover, Err> Stream
    for Executor<S, PairId, StableId, Ver, O, P, B, Txc, Tx, C, Index, Cache, Book, Ir, Prover, Err>
where
    S: Stream<Item = (PairId, EitherMod<StateUpdate<Bundled<Either<O, P>, B>>>)> + Unpin,
    PairId: Copy + Eq + Ord + Hash + Display + Unpin,
    StableId: Copy + Eq + Hash + Display + Unpin,
    Ver: Copy + Eq + Hash + Display + Unpin,
    P: EntitySnapshot<StableId = StableId, Version = Ver> + Copy + Unpin,
    O: EntitySnapshot<StableId = StableId, Version = Ver> + Fragment + OrderState + Copy + Unpin,
    B: Clone + Unpin,
    Txc: Unpin,
    Tx: Unpin,
    C: Clone + Unpin,
    Index: StateIndex<Bundled<Either<O, P>, B>> + Unpin,
    Cache: StateIndexCache<StableId, Bundled<Either<O, P>, B>> + Unpin,
    Book: TemporalLiquidityBook<O, P> + ExternalTLBEvents<O, P> + TLBFeedback<O, P> + Maker<C> + Unpin,
    Ir: RecipeInterpreter<O, P, C, B, Txc> + Unpin,
    Prover: TxProver<Txc, Tx> + Unpin,
    Err: Unpin,
{
    type Item = Tx;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context) -> Poll<Option<Self::Item>> {
        loop {
            // Wait for the feedback from the last pending job.
            if let Some((pair, mut pending_effects)) = self.pending_effects.take() {
                if let Poll::Ready(Some(result)) = Stream::poll_next(Pin::new(&mut self.feedback), cx) {
                    match result {
                        Ok(_) => {
                            while let Some((e, bearer)) = pending_effects.pop() {
                                self.update_state(EitherMod::Unconfirmed(Unconfirmed(
                                    StateUpdate::Transition(Ior::Right(Bundled(e, bearer))),
                                )));
                            }
                            self.multi_book.get_mut(&pair).on_recipe_succeeded();
                        }
                        Err(_err) => {
                            // todo: invalidate missing bearers.
                            self.multi_book.get_mut(&pair).on_recipe_failed();
                        }
                    }
                    continue;
                }
                let _ = self.pending_effects.insert((pair, pending_effects));
            }
            // Prioritize external updates over local work.
            if let Poll::Ready(Some((pair, update))) = Stream::poll_next(Pin::new(&mut self.upstream), cx) {
                self.sync_book(pair, update);
                self.focus_set.insert(pair);
                continue;
            }
            // Finally attempt to execute something.
            while let Some(focus_pair) = self.focus_set.pop_first() {
                if let Some(recipe) = self.multi_book.get_mut(&focus_pair).attempt() {
                    let linked_recipe = self.link_recipe(recipe.into());
                    let ctx = self.context.clone();
                    let (txc, effects) = self.interpreter.run(linked_recipe, ctx);
                    let _ = self.pending_effects.insert((focus_pair, effects));
                    let tx = self.prover.prove(txc);
                    // Return pair to focus set to make sure corresponding TLB will be exhausted.
                    self.focus_set.insert(focus_pair);
                    return Poll::Ready(Some(tx));
                }
            }
            return Poll::Pending;
        }
    }
}

impl<S, PairId, StableId, Ver, O, P, B, Txc, Tx, C, Index, Cache, Book, Ir, Prover, Err> FusedStream
    for Executor<S, PairId, StableId, Ver, O, P, B, Txc, Tx, C, Index, Cache, Book, Ir, Prover, Err>
where
    S: Stream<Item = (PairId, EitherMod<StateUpdate<Bundled<Either<O, P>, B>>>)> + Unpin,
    PairId: Copy + Eq + Ord + Hash + Display + Unpin,
    StableId: Copy + Eq + Hash + Display + Unpin,
    Ver: Copy + Eq + Hash + Display + Unpin,
    P: EntitySnapshot<StableId = StableId, Version = Ver> + Copy + Unpin,
    O: EntitySnapshot<StableId = StableId, Version = Ver> + Fragment + OrderState + Copy + Unpin,
    B: Clone + Unpin,
    Txc: Unpin,
    Tx: Unpin,
    C: Clone + Unpin,
    Index: StateIndex<Bundled<Either<O, P>, B>> + Unpin,
    Cache: StateIndexCache<StableId, Bundled<Either<O, P>, B>> + Unpin,
    Book: TemporalLiquidityBook<O, P> + ExternalTLBEvents<O, P> + TLBFeedback<O, P> + Maker<C> + Unpin,
    Ir: RecipeInterpreter<O, P, C, B, Txc> + Unpin,
    Prover: TxProver<Txc, Tx> + Unpin,
    Err: Unpin,
{
    fn is_terminated(&self) -> bool {
        false
    }
}
