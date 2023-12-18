use futures::future::Either;
use futures::Stream;
use log::trace;

use bloom_offchain::execution_engine::bundled::Bundled;
use bloom_offchain::execution_engine::liquidity_book::{ExternalTLBEvents, TemporalLiquidityBook};
use bloom_offchain::execution_engine::resolver::resolve_source_state;
use bloom_offchain::execution_engine::storage::cache::StateIndexCache;
use bloom_offchain::execution_engine::storage::StateIndex;
use spectrum_cardano_lib::output::FinalizedTxOut;
use spectrum_offchain::combinators::Ior;
use spectrum_offchain::data::LiquiditySource;
use spectrum_offchain::data::unique_entity::{Confirmed, EitherMod, StateUpdate, Unconfirmed};

mod interpreter;

pub struct Executor {}

type Entity<O, P> = Bundled<Either<O, P>, FinalizedTxOut>;

fn sync_book<O, P, Src, Index, Cache, Book>(
    index: &mut Index,
    cache: &mut Cache,
    book: &mut Book,
    update: EitherMod<StateUpdate<Bundled<Either<O, P>, Src>>>,
) where
    Src: Clone,
    Either<O, P>: LiquiditySource + Clone,
    Index: StateIndex<Bundled<Either<O, P>, Src>>,
    Cache: StateIndexCache<Bundled<Either<O, P>, Src>>,
    Book: ExternalTLBEvents<O, P>,
{
    match update_state(index, cache, update) {
        None => {}
        Some(Ior::Left(e)) => match e {
            Either::Left(o) => book.remove_fragment(o),
            Either::Right(p) => book.remove_pool(p),
        },
        Some(Ior::Both(old, new)) => match (old, new) {
            (Either::Left(old), Either::Left(new)) => {
                book.remove_fragment(old);
                book.add_fragment(new);
            }
            (_, Either::Right(new)) => {
                book.update_pool(new);
            }
            _ => unreachable!(),
        },
        Some(Ior::Right(new)) => match new {
            Either::Left(new) => book.add_fragment(new),
            Either::Right(new) => book.update_pool(new),
        },
    }
}

fn update_state<T, Src, Index, Cache>(
    index: &mut Index,
    cache: &mut Cache,
    update: EitherMod<StateUpdate<Bundled<T, Src>>>,
) -> Option<Ior<T, T>>
where
    T: LiquiditySource + Clone,
    Src: Clone,
    Index: StateIndex<Bundled<T, Src>>,
    Cache: StateIndexCache<Bundled<T, Src>>,
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
                trace!("Observing new confirmed state of pool {}", id);
                index.put_confirmed(Confirmed(new_state));
            } else {
                trace!("Observing new unconfirmed state of pool {}", id);
                index.put_unconfirmed(Unconfirmed(new_state));
            }
            match resolve_source_state(id, index) {
                Some(latest_state) => {
                    if let Some(Bundled(prev_best_state, _)) = cache.insert(latest_state.clone()) {
                        Some(Ior::Both(prev_best_state, latest_state.0))
                    } else {
                        Some(Ior::Right(latest_state.0))
                    }
                }
                None => unreachable!(),
            }
        }
        StateUpdate::Transition(Ior::Left(st)) => {
            index.eliminate(st.version(), st.stable_id());
            Some(Ior::Left(st.0))
        }
        StateUpdate::TransitionRollback(Ior::Left(st)) => {
            let id = st.stable_id();
            trace!(target: "offchain", "Rolling back state of pool {}", id);
            index.invalidate(st.version(), id);
            Some(Ior::Left(st.0))
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn foo() {}
}
