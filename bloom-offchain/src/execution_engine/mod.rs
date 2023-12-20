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
use spectrum_offchain::data::{Has, LiquiditySource};
use spectrum_offchain::network::Network;

use crate::execution_engine::bundled::Bundled;
use crate::execution_engine::interpreter::RecipeInterpreter;
use crate::execution_engine::liquidity_book::fragment::{Fragment, OrderState};
use crate::execution_engine::liquidity_book::recipe::{
    ExecutionRecipe, LinkedExecutionRecipe, LinkedFill, LinkedSwap, LinkedTerminalInstruction,
    TerminalInstruction,
};
use crate::execution_engine::liquidity_book::{ExternalTLBEvents, TLBFeedback, TemporalLiquidityBook};
use crate::execution_engine::resolver::resolve_source_state;
use crate::execution_engine::storage::cache::StateIndexCache;
use crate::execution_engine::storage::StateIndex;

pub mod batch_exec;
pub mod bundled;
pub mod interpreter;
pub mod liquidity_book;
pub mod partial_fill;
pub mod resolver;
pub mod storage;
pub mod types;

pub fn execution_stream<'a, Id, V, O, P, B, Tx, Ctx, Index, Cache, Book, Interpreter, Net, Err>(
    executor: Executor<Id, V, O, P, B, Tx, Ctx, Index, Cache, Book, Interpreter, Err>,
    feedback: mpsc::Sender<Result<(), Err>>,
    network: Net,
) -> impl Stream<Item = ()> + 'a
where
    Id: Copy + Eq + Hash + Display + Unpin + 'a,
    V: Copy + Eq + Hash + Display + Unpin + 'a,
    P: LiquiditySource<StableId = Id, Version = V> + Copy + Unpin + 'a,
    O: LiquiditySource<StableId = Id, Version = V> + Fragment + OrderState + Copy + Unpin + 'a,
    B: Clone + Unpin + 'a,
    Tx: Unpin + 'a,
    Ctx: Copy + Unpin + 'a,
    Index: StateIndex<Bundled<Either<O, P>, B>> + Unpin + 'a,
    Cache: StateIndexCache<Id, Bundled<Either<O, P>, B>> + Unpin + 'a,
    Book: TemporalLiquidityBook<O, P> + ExternalTLBEvents<O, P> + TLBFeedback<O, P> + Unpin + 'a,
    Interpreter: RecipeInterpreter<O, P, Ctx, B, Tx> + Unpin + 'a,
    Net: Network<Tx, Err> + Clone + 'a,
    Err: Unpin + 'a,
{
    executor.then(move |tx| {
        let mut network = network.clone();
        let mut feedback = feedback.clone();
        async move {
            let result = network.submit_tx(tx).await;
            feedback.send(result).await.expect("Filed to propagate feedback.");
        }
    })
}

pub struct Executor<StableId, Version, O, P, Bearer, Tx, Ctx, Index, Cache, Book, Interpreter, Err> {
    index: Index,
    cache: Cache,
    book: Book,
    context: Ctx,
    interpreter: Interpreter,
    upstream: mpsc::Receiver<EitherMod<StateUpdate<Bundled<Either<O, P>, Bearer>>>>,
    feedback: mpsc::Receiver<Result<(), Err>>,
    pending_effects: Option<Vec<(Either<O, P>, Bearer)>>,
    pd1: PhantomData<StableId>,
    pd2: PhantomData<Version>,
    pd3: PhantomData<Tx>,
    pd4: PhantomData<Err>,
}

impl<StableId, Version, O, P, Bearer, Tx, Ctx, Index, Cache, Book, Interpreter, Err>
    Executor<StableId, Version, O, P, Bearer, Tx, Ctx, Index, Cache, Book, Interpreter, Err>
{
    fn sync_book(&mut self, update: EitherMod<StateUpdate<Bundled<Either<O, P>, Bearer>>>)
    where
        StableId: Copy + Eq + Hash + Display,
        Version: Copy + Eq + Hash + Display,
        Bearer: Clone,
        O: LiquiditySource<StableId = StableId, Version = Version> + Clone,
        P: LiquiditySource<StableId = StableId, Version = Version> + Clone,
        Index: StateIndex<Bundled<Either<O, P>, Bearer>>,
        Cache: StateIndexCache<StableId, Bundled<Either<O, P>, Bearer>>,
        Book: ExternalTLBEvents<O, P>,
    {
        match self.update_state(update) {
            None => {}
            Some(Ior::Left(e)) => match e {
                Either::Left(o) => self.book.remove_fragment(o),
                Either::Right(p) => self.book.remove_pool(p),
            },
            Some(Ior::Both(old, new)) => match (old, new) {
                (Either::Left(old), Either::Left(new)) => {
                    self.book.remove_fragment(old);
                    self.book.add_fragment(new);
                }
                (_, Either::Right(new)) => {
                    self.book.update_pool(new);
                }
                _ => unreachable!(),
            },
            Some(Ior::Right(new)) => match new {
                Either::Left(new) => self.book.add_fragment(new),
                Either::Right(new) => self.book.update_pool(new),
            },
        }
    }

    fn update_state<T>(&mut self, update: EitherMod<StateUpdate<Bundled<T, Bearer>>>) -> Option<Ior<T, T>>
    where
        StableId: Copy + Eq + Hash + Display,
        Version: Copy + Eq + Hash + Display,
        T: LiquiditySource<StableId = StableId> + Clone,
        Bearer: Clone,
        Index: StateIndex<Bundled<T, Bearer>>,
        Cache: StateIndexCache<StableId, Bundled<T, Bearer>>,
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

    fn link_recipe(
        &self,
        ExecutionRecipe(mut xs): ExecutionRecipe<O, P>,
    ) -> LinkedExecutionRecipe<O, P, Bearer>
    where
        StableId: Copy + Eq + Hash + Display,
        Version: Copy + Eq + Hash + Display,
        O: LiquiditySource<StableId = StableId, Version = Version>,
        P: LiquiditySource<StableId = StableId, Version = Version>,
        Cache: StateIndexCache<StableId, Bundled<Either<O, P>, Bearer>>,
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

impl<StableId, Version, O, P, Bearer, Tx, Ctx, Index, Cache, Book, Interpreter, Err> Stream
    for Executor<StableId, Version, O, P, Bearer, Tx, Ctx, Index, Cache, Book, Interpreter, Err>
where
    StableId: Copy + Eq + Hash + Display + Unpin,
    Version: Copy + Eq + Hash + Display + Unpin,
    P: LiquiditySource<StableId = StableId, Version = Version> + Copy + Unpin,
    O: LiquiditySource<StableId = StableId, Version = Version> + Fragment + OrderState + Copy + Unpin,
    Bearer: Clone + Unpin,
    Tx: Unpin,
    Ctx: Copy + Unpin,
    Index: StateIndex<Bundled<Either<O, P>, Bearer>> + Unpin,
    Cache: StateIndexCache<StableId, Bundled<Either<O, P>, Bearer>> + Unpin,
    Book: TemporalLiquidityBook<O, P> + ExternalTLBEvents<O, P> + TLBFeedback<O, P> + Unpin,
    Interpreter: RecipeInterpreter<O, P, Ctx, Bearer, Tx> + Unpin,
    Err: Unpin,
{
    type Item = Tx;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context) -> Poll<Option<Self::Item>> {
        loop {
            // Wait for the feedback from the last pending job.
            if let Some(mut pending_effects) = self.pending_effects.take() {
                if let Poll::Ready(Some(result)) = Stream::poll_next(Pin::new(&mut self.feedback), cx) {
                    match result {
                        Ok(_) => {
                            while let Some((e, bearer)) = pending_effects.pop() {
                                self.update_state(EitherMod::Unconfirmed(Unconfirmed(
                                    StateUpdate::Transition(Ior::Right(Bundled(e, bearer))),
                                )));
                            }
                            self.book.on_recipe_succeeded();
                        }
                        Err(_err) => {
                            // todo: invalidate missing bearers.
                            self.book.on_recipe_failed();
                        }
                    }
                    continue;
                }
                let _ = self.pending_effects.insert(pending_effects);
            }
            // Prioritize external updates over local work.
            if let Poll::Ready(Some(update)) = Stream::poll_next(Pin::new(&mut self.upstream), cx) {
                self.sync_book(update);
                continue;
            }
            // Finally attempt to execute something.
            if let Some(recipe) = self.book.attempt() {
                let linked_recipe = self.link_recipe(recipe.into());
                let ctx = self.context.clone();
                let (tx, effects) = self.interpreter.run(linked_recipe, ctx);
                let _ = self.pending_effects.insert(effects);
                return Poll::Ready(Some(tx));
            }
            return Poll::Pending;
        }
    }
}

impl<StableId, Version, O, P, Bearer, Tx, Ctx, Index, Cache, Book, Interpreter, Err> FusedStream
    for Executor<StableId, Version, O, P, Bearer, Tx, Ctx, Index, Cache, Book, Interpreter, Err>
where
    StableId: Copy + Eq + Hash + Display + Unpin,
    Version: Copy + Eq + Hash + Display + Unpin,
    P: LiquiditySource<StableId = StableId, Version = Version> + Copy + Unpin,
    O: LiquiditySource<StableId = StableId, Version = Version> + Fragment + OrderState + Copy + Unpin,
    Bearer: Clone + Unpin,
    Tx: Unpin,
    Ctx: Copy + Unpin,
    Index: StateIndex<Bundled<Either<O, P>, Bearer>> + Unpin,
    Cache: StateIndexCache<StableId, Bundled<Either<O, P>, Bearer>> + Unpin,
    Book: TemporalLiquidityBook<O, P> + ExternalTLBEvents<O, P> + TLBFeedback<O, P> + Unpin,
    Interpreter: RecipeInterpreter<O, P, Ctx, Bearer, Tx> + Unpin,
    Err: Unpin,
{
    fn is_terminated(&self) -> bool {
        false
    }
}
