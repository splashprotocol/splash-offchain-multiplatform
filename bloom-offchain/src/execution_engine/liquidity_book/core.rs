use std::cmp::{min, Ordering};
use std::collections::HashMap;
use std::ops::AddAssign;

use either::Either;

use algebra_core::monoid::Monoid;
use algebra_core::semigroup::Semigroup;
use spectrum_offchain::data::Stable;

use crate::execution_engine::liquidity_book::fragment::Fragment;
use crate::execution_engine::liquidity_book::market_maker::MarketMaker;
use crate::execution_engine::liquidity_book::side::{Side, SideM};
use crate::execution_engine::liquidity_book::types::{FeeAsset, InputAsset, OutputAsset};

/// Taking liquidity from market.
#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub struct TerminalTake {
    /// Input asset removed as a result of this transaction.
    pub removed_input: InputAsset<u64>,
    /// Output asset added as a result of this transaction.
    pub added_output: OutputAsset<u64>,
    /// Overall execution budget used.
    pub budget_used: FeeAsset<u64>,
    /// Execution fee charged.
    pub fee_used: FeeAsset<u64>,
}

impl TerminalTake {
    pub fn new() -> Self {
        Self {
            removed_input: 0,
            added_output: 0,
            budget_used: 0,
            fee_used: 0,
        }
    }
}

#[derive(Debug, Copy, Clone)]
pub enum Next<S, T> {
    /// Successive state is available.
    Succ(S),
    /// Terminal state.
    Term(T),
}

impl<S, T> Next<S, T> {
    pub fn map_succ<S1, F>(self, f: F) -> Next<S1, T>
    where
        F: FnOnce(S) -> S1,
    {
        match self {
            Next::Succ(succ) => Next::Succ(f(succ)),
            Next::Term(term) => Next::Term(term),
        }
    }

    pub fn fold<R, F1, F2>(self, f1: F1, f2: F2) -> R
    where
        F1: FnOnce(S) -> R,
        F2: FnOnce(T) -> R,
    {
        match self {
            Next::Succ(succ) => f1(succ),
            Next::Term(term) => f2(term),
        }
    }

    pub fn fold_ref<R, F1, F2>(&self, f1: F1, f2: F2) -> R
    where
        F1: FnOnce(&S) -> R,
        F2: FnOnce(&T) -> R,
    {
        match self {
            Next::Succ(succ) => f1(succ),
            Next::Term(term) => f2(term),
        }
    }
}

/// State transition of a take.
#[derive(Debug, Copy, Clone)]
pub struct Trans<Cont, Term> {
    pub target: Cont,
    pub result: Next<Cont, Term>,
}

impl<Cont, Term> Semigroup for Trans<Cont, Term> {
    fn combine(self, other: Self) -> Self {
        Self {
            target: self.target,
            result: other.result,
        }
    }
}

pub type TakerTrans<Taker> = Trans<Taker, TerminalTake>;

pub type MakerTrans<Maker> = Trans<Maker, ()>;

impl<Maker> MakerTrans<Maker> {
    pub fn trade_side(&self) -> Option<SideM>
    where
        Maker: MarketMaker,
    {
        match &self.result {
            Next::Succ(succ) => match self.target.static_price().cmp(&succ.static_price()) {
                Ordering::Less => Some(SideM::Ask),
                Ordering::Equal => None,
                Ordering::Greater => Some(SideM::Bid),
            },
            _ => None,
        }
    }

    pub fn gain(&self) -> Option<Side<u64>>
    where
        Maker: MarketMaker,
    {
        match &self.result {
            Next::Succ(succ) => {
                let (succ_reserves_b, succ_reserves_q) = succ.liquidity();
                let (init_reserved_b, init_reserved_q) = self.target.liquidity();
                succ_reserves_b
                    .checked_sub(init_reserved_b)
                    .map(Side::Ask)
                    .or_else(|| succ_reserves_q.checked_sub(init_reserved_q).map(Side::Bid))
            }
            _ => None,
        }
    }
}

impl<T> TakerTrans<T> {
    pub fn added_output(&self) -> OutputAsset<u64>
    where
        T: Fragment,
    {
        let accumulated_output = match &self.result {
            Next::Succ(next) => next.output(),
            Next::Term(term) => term.added_output,
        };
        accumulated_output - self.target.output()
    }
}

#[derive(Debug, Clone)]
pub struct MatchmakingAttempt<Taker: Stable, Maker: Stable, U> {
    takes: HashMap<Taker::StableId, TakerTrans<Taker>>,
    makes: HashMap<Maker::StableId, MakerTrans<Maker>>,
    execution_units_consumed: U,
}

impl<Taker: Stable, Maker: Stable, U> MatchmakingAttempt<Taker, Maker, U> {
    pub fn empty() -> Self
    where
        U: Monoid,
    {
        Self {
            takes: HashMap::new(),
            makes: HashMap::new(),
            execution_units_consumed: U::empty(),
        }
    }

    pub fn is_complete(&self) -> bool {
        self.takes.len() > 1 || self.takes.len() == 1 && self.makes.len() > 0
    }

    pub fn execution_units_consumed(&self) -> U
    where
        U: Copy,
    {
        self.execution_units_consumed
    }

    pub fn unsatisfied_fragments(&self) -> Vec<Taker>
    where
        Taker: Fragment + Copy,
    {
        self.takes
            .iter()
            .filter_map(|(_, apply)| {
                let target = apply.target;
                if apply.added_output() < target.min_marginal_output() {
                    Some(target)
                } else {
                    None
                }
            })
            .collect()
    }

    pub fn next_offered_chunk(&self, taker: &Taker) -> Side<u64>
    where
        Taker: Fragment,
    {
        let initial_state = self
            .takes
            .get(&taker.stable_id())
            .map(|tr| &tr.target)
            .unwrap_or(taker);
        let initial_chunk = initial_state.input() * 10 / 100;
        let chunk = if initial_chunk > 0 {
            min(initial_chunk, taker.input())
        } else {
            taker.input()
        };
        taker.side().wrap(chunk)
    }

    pub fn add_take(&mut self, take: TakerTrans<Taker>)
    where
        Taker: Fragment<U = U>,
        U: AddAssign,
    {
        let sid = take.target.stable_id();
        let take_combined = match self.takes.remove(&sid) {
            None => {
                self.execution_units_consumed += take.target.marginal_cost_hint();
                take
            }
            Some(existing_transition) => existing_transition.combine(take),
        };
        self.takes.insert(sid, take_combined);
    }

    pub fn add_make(&mut self, make: MakerTrans<Maker>) -> Result<(), ()>
    where
        Maker: MarketMaker<U = U>,
        U: AddAssign,
    {
        let sid = make.target.stable_id();
        let maker_combined = match self.makes.remove(&sid) {
            None => {
                self.execution_units_consumed += make.target.marginal_cost_hint();
                make
            }
            Some(accumulated_trans) => {
                if accumulated_trans.trade_side() == make.trade_side() {
                    accumulated_trans.combine(make)
                } else {
                    return Err(());
                }
            }
        };
        self.makes.insert(sid, maker_combined);
        Ok(())
    }
}

#[derive(Debug, Copy, Clone)]
pub struct Applied<Action, Subject: Stable> {
    pub action: Action,
    pub target: Subject::StableId,
    pub result: Next<Subject, ()>,
}

#[derive(Debug, Clone)]
pub struct MatchmakingRecipe<Taker: Stable, Maker: Stable> {
    instructions: Vec<Either<TakerTrans<Taker>, MakerTrans<Maker>>>,
}

impl<Taker, Maker> MatchmakingRecipe<Taker, Maker>
where
    Taker: Stable,
    Maker: Stable,
{
    pub fn try_from<U>(attempt: MatchmakingAttempt<Taker, Maker, U>) -> Result<Self, Option<Vec<Taker>>>
    where
        Taker: Fragment + Copy,
    {
        if attempt.is_complete() {
            let unsatisfied_fragments = attempt.unsatisfied_fragments();
            if unsatisfied_fragments.is_empty() {
                let MatchmakingAttempt { takes, makes, .. } = attempt;
                let mut instructions = vec![];
                for take in takes.into_values() {
                    instructions.push(Either::Left(take));
                }
                for make in makes.into_values() {
                    instructions.push(Either::Right(make));
                }
                Ok(Self { instructions })
            } else {
                Err(Some(unsatisfied_fragments))
            }
        } else {
            Err(None)
        }
    }
}
