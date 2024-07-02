use std::cmp::{max, min, Ordering};
use std::collections::HashMap;
use std::mem;
use std::ops::AddAssign;

use either::Either;
use num_rational::Ratio;

use algebra_core::monoid::Monoid;
use algebra_core::semigroup::Semigroup;
use spectrum_offchain::data::Stable;

use crate::execution_engine::bundled::Bundled;
use crate::execution_engine::liquidity_book::fragment::{Fragment, TakerBehaviour};
use crate::execution_engine::liquidity_book::market_maker::MarketMaker;
use crate::execution_engine::liquidity_book::side::{Side, SideM};
use crate::execution_engine::liquidity_book::types::{FeeAsset, InputAsset, OutputAsset};

/// Taking liquidity from market.
#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub struct TerminalTake {
    /// Input asset removed as a result of this transaction.
    pub remaining_input: InputAsset<u64>,
    /// Output asset added as a result of this transaction.
    pub accumulated_output: OutputAsset<u64>,
    /// Remaining execution budget.
    pub remaining_budget: FeeAsset<u64>,
    /// Remaining operator fee.
    pub remaining_fee: FeeAsset<u64>,
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
}

/// State transition of a take.
#[derive(Debug, Copy, Clone)]
pub struct Trans<Init, Cont, Term> {
    pub target: Init,
    pub result: Next<Cont, Term>,
}

impl<Cont, Term> Trans<Cont, Cont, Term> {
    pub fn map<B, F>(self, f: F) -> Trans<B, B, Term>
    where
        F: Fn(Cont) -> B,
    {
        Trans {
            target: f(self.target),
            result: self.result.map_succ(f),
        }
    }
}

impl<Init, Cont, Term> Trans<Init, Cont, Term> {
    pub fn map_cont<B, F>(self, f: F) -> Trans<Init, B, Term>
    where
        F: FnOnce(Cont) -> B,
    {
        Trans {
            target: self.target,
            result: self.result.map_succ(f),
        }
    }
}

impl<Init, Cont, Term> Semigroup for Trans<Init, Cont, Term> {
    fn combine(self, other: Self) -> Self {
        Self {
            target: self.target,
            result: other.result,
        }
    }
}

pub type TakeInProgress<Taker> = Trans<Taker, Taker, TerminalTake>;

pub type Take<Taker, Bearer> = Trans<Bundled<Taker, Bearer>, Taker, TerminalTake>;

pub type MakeInProgress<Maker> = Trans<Maker, Maker, ()>;

pub type Make<Maker, Bearer> = Trans<Bundled<Maker, Bearer>, Maker, ()>;

impl<T, B> Take<T, B> {
    pub fn added_output(&self) -> OutputAsset<u64>
    where
        T: Fragment,
    {
        let accumulated_output = match &self.result {
            Next::Succ(next) => next.output(),
            Next::Term(term) => term.accumulated_output,
        };
        accumulated_output
            .checked_sub(self.target.0.output())
            .expect("Output cannot decrease")
    }

    pub fn removed_input(&self) -> InputAsset<u64>
    where
        T: Fragment,
    {
        let remaining_input = match &self.result {
            Next::Succ(next) => next.input(),
            Next::Term(term) => term.remaining_input,
        };
        self.target
            .0
            .input()
            .checked_sub(remaining_input)
            .expect("Input cannot increase")
    }

    pub fn consumed_fee(&self) -> FeeAsset<u64>
    where
        T: Fragment,
    {
        let remaining_fee = match &self.result {
            Next::Succ(next) => next.fee(),
            Next::Term(term) => term.remaining_fee,
        };
        self.target
            .0
            .fee()
            .checked_sub(remaining_fee)
            .expect("Fee cannot increase")
    }

    pub fn consumed_budget(&self) -> FeeAsset<u64>
    where
        T: Fragment,
    {
        let remaining_budget = match &self.result {
            Next::Succ(next) => next.budget(),
            Next::Term(term) => term.remaining_budget,
        };
        self.target
            .0
            .budget()
            .checked_sub(remaining_budget)
            .expect("Budget cannot increase")
    }

    pub fn scale_budget(&mut self, scale: Ratio<u64>) -> i64
    where
        T: Fragment + TakerBehaviour + Copy,
    {
        match &mut self.result {
            Next::Succ(mut next) => {
                let old_val = self.consumed_budget();
                let new_val = old_val * scale.numer() / scale.denom();
                let delta = old_val as i64 - new_val as i64;
                let (_, updated) = next.with_budget_corrected(delta);
                let _ = mem::replace(&mut next, updated);
                delta
            }
            Next::Term(mut term) => {
                let old_val = self.consumed_budget();
                let new_val = old_val * scale.numer() / scale.denom();
                let delta = old_val as i64 - new_val as i64;
                term.remaining_budget = (term.remaining_budget as i64 + delta) as u64;
                delta
            }
        }
    }

    pub fn correct_budget(&mut self, delta: i64) -> i64
    where
        T: Fragment + TakerBehaviour + Copy,
    {
        match &mut self.result {
            Next::Succ(mut next) => {
                let (real_delta, updated) = next.with_budget_corrected(delta);
                let _ = mem::replace(&mut next, updated);
                real_delta
            }
            Next::Term(mut term) => {
                let old_val = self.consumed_budget() as i64;
                let new_val = max(old_val + delta, 0);
                let real_delta = old_val - new_val;
                term.remaining_budget = (term.remaining_budget as i64 + real_delta) as u64;
                real_delta
            }
        }
    }
}

impl<Maker, B> Make<Maker, B> {
    pub fn trade_side(&self) -> Option<SideM>
    where
        Maker: MarketMaker,
    {
        match &self.result {
            Next::Succ(succ) => match self.target.0.static_price().cmp(&succ.static_price()) {
                Ordering::Less => Some(SideM::Ask),
                Ordering::Equal => None,
                Ordering::Greater => Some(SideM::Bid),
            },
            _ => None,
        }
    }

    pub fn gain(&self) -> Option<u64>
    where
        Maker: MarketMaker,
    {
        match &self.result {
            Next::Succ(succ) => {
                let (succ_reserves_b, succ_reserves_q) = succ.liquidity();
                let (init_reserved_b, init_reserved_q) = self.target.0.liquidity();
                succ_reserves_b
                    .checked_sub(init_reserved_b)
                    .or_else(|| succ_reserves_q.checked_sub(init_reserved_q))
            }
            _ => None,
        }
    }

    pub fn loss(&self) -> Option<u64>
    where
        Maker: MarketMaker,
    {
        match &self.result {
            Next::Succ(succ) => {
                let (succ_reserves_b, succ_reserves_q) = succ.liquidity();
                let (init_reserves_b, init_reserves_q) = self.target.0.liquidity();
                init_reserves_b
                    .checked_sub(succ_reserves_b)
                    .or_else(|| init_reserves_q.checked_sub(succ_reserves_q))
            }
            _ => None,
        }
    }
}

impl<Maker> MakeInProgress<Maker> {
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

    pub fn loss(&self) -> Option<Side<u64>>
    where
        Maker: MarketMaker,
    {
        match &self.result {
            Next::Succ(succ) => {
                let (succ_reserves_b, succ_reserves_q) = succ.liquidity();
                let (init_reserves_b, init_reserves_q) = self.target.liquidity();
                init_reserves_b
                    .checked_sub(succ_reserves_b)
                    .map(Side::Bid)
                    .or_else(|| init_reserves_q.checked_sub(succ_reserves_q).map(Side::Ask))
            }
            _ => None,
        }
    }
}

impl<T> TakeInProgress<T> {
    pub fn added_output(&self) -> OutputAsset<u64>
    where
        T: Fragment,
    {
        let accumulated_output = match &self.result {
            Next::Succ(next) => next.output(),
            Next::Term(term) => term.accumulated_output,
        };
        accumulated_output
            .checked_sub(self.target.output())
            .expect("Output cannot decrease")
    }
}

#[derive(Debug, Clone)]
pub struct MatchmakingAttempt<Taker: Stable, Maker: Stable, U> {
    takes: HashMap<Taker::StableId, TakeInProgress<Taker>>,
    makes: HashMap<Maker::StableId, MakeInProgress<Maker>>,
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

    pub fn add_take(&mut self, take: TakeInProgress<Taker>)
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

    pub fn add_make(&mut self, make: MakeInProgress<Maker>) -> Result<(), ()>
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
pub struct MatchmakingRecipe<Taker, Maker> {
    pub(crate) instructions: Vec<Either<TakeInProgress<Taker>, MakeInProgress<Maker>>>,
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

pub type Execution<T, M, B> = Either<Take<T, B>, Make<M, B>>;

// impl<T, M, B> Execution<T, M, B> {
//     pub fn scale_budget(&mut self, scale: Ratio<u64>) -> i64 {
//         match self {
//             LinkedTerminalInstruction::Fill(fill) => {
//                 let old_val = fill.budget_used;
//                 let new_val = fill.budget_used * scale.numer() / scale.denom();
//                 fill.budget_used = new_val;
//                 let delta = new_val as i64 - old_val as i64;
//                 delta
//             }
//             LinkedTerminalInstruction::Swap(_) => 0,
//         }
//     }
//
//     pub fn correct_budget(&mut self, val: i64) -> i64 {
//         match self {
//             LinkedTerminalInstruction::Fill(fill) => {
//                 let old_val = fill.budget_used as i64;
//                 let new_val = max(old_val + val, 0);
//                 fill.budget_used = new_val as u64;
//                 let delta = new_val - old_val;
//                 delta
//             }
//             LinkedTerminalInstruction::Swap(_) => 0,
//         }
//     }
// }

/// Same as [MatchmakingRecipe] but with bearers attached to each target element.
#[derive(Debug, Clone)]
pub struct ExecutionRecipe<Taker, Maker, B>(pub Vec<Execution<Taker, Maker, B>>);

impl<T, M, B> ExecutionRecipe<T, M, B> {
    pub fn new<I, F>(MatchmakingRecipe { instructions }: MatchmakingRecipe<T, M>, link: F) -> Result<Self, ()>
    where
        T: Stable<StableId = I>,
        M: Stable<StableId = I>,
        F: Fn(I) -> Option<B>,
    {
        let mut translated_instructions = vec![];
        for i in instructions {
            match i {
                Either::Left(Trans { target, result }) => {
                    if let Some(bearer) = link(target.stable_id()) {
                        translated_instructions.push(Either::Left(Trans {
                            target: Bundled(target, bearer),
                            result,
                        }));
                        continue;
                    }
                }
                Either::Right(Trans { target, result }) => {
                    if let Some(bearer) = link(target.stable_id()) {
                        translated_instructions.push(Either::Right(Trans {
                            target: Bundled(target, bearer),
                            result,
                        }));
                        continue;
                    }
                }
            }
            return Err(());
        }
        Ok(Self(translated_instructions))
    }
}
