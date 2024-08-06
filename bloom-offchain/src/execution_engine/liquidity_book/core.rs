use derive_more::Display;
use either::Either;
use log::trace;
use num_rational::Ratio;
use std::cmp::{max, min, Ordering};
use std::collections::{HashMap, HashSet};
use std::fmt::{Display, Formatter, Write};
use std::hash::Hash;
use std::mem;
use std::ops::AddAssign;

use algebra_core::monoid::Monoid;
use algebra_core::semigroup::Semigroup;
use spectrum_offchain::data::Stable;

use crate::execution_engine::bundled::Bundled;
use crate::execution_engine::liquidity_book::fragment::{MarketTaker, TakerBalance, TakerBehaviour};
use crate::execution_engine::liquidity_book::market_maker::{
    AbsoluteReserves, Excess, MakerBalance, MarketMaker,
};
use crate::execution_engine::liquidity_book::side::{OnSide, Side};
use crate::execution_engine::liquidity_book::types::{FeeAsset, InputAsset, OutputAsset};

/// Terminal state of a take that was fulfilled.
#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub struct TerminalTake {
    /// Remainder of input asset.
    pub remaining_input: InputAsset<u64>,
    /// Output asset added as a result of this transaction.
    pub accumulated_output: OutputAsset<u64>,
    /// Remaining execution budget.
    pub remaining_budget: FeeAsset<u64>,
    /// Remaining operator fee.
    pub remaining_fee: FeeAsset<u64>,
}

impl TerminalTake {
    pub fn with_budget_corrected(mut self, delta: i64) -> (i64, Self) {
        let budget_remainder = self.remaining_budget as i64;
        let corrected_remainder = budget_remainder + delta;
        let updated_budget_remainder = max(corrected_remainder, 0);
        let real_delta = updated_budget_remainder - budget_remainder;
        self.remaining_budget = updated_budget_remainder as u64;
        (real_delta, self)
    }
}

impl TakerBalance for TerminalTake {
    fn balance(mut self, added_output: u64) -> Self {
        self.accumulated_output += added_output;
        self
    }
}

impl Display for TerminalTake {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(format!("TerminalTake(remaining_input={}, accumulated_output={}, remaining_budget={}, remaining_fee={})", self.remaining_input, self.accumulated_output, self.remaining_budget, self.remaining_fee).as_str())
    }
}

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub enum Next<S, T> {
    /// Successive state is available.
    Succ(S),
    /// Terminal state.
    Term(T),
}

impl<S: Display, T: Display> Display for Next<S, T> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Next::Succ(s) => f.write_str(format!("Succ({})", s).as_str()),
            Next::Term(t) => f.write_str(format!("Term({})", t).as_str()),
        }
    }
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
#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub struct Trans<Init, Cont, Term> {
    pub target: Init,
    pub result: Next<Cont, Term>,
}

impl<I: Display, C: Display, T: Display> Display for Trans<I, C, T> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(format!("{} -> {}", self.target, self.result).as_str())
    }
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
    pub fn new(target: Init, result: Next<Cont, Term>) -> Self {
        Self { target, result }
    }

    pub fn map_target<B, F>(self, f: F) -> Trans<B, Cont, Term>
    where
        F: FnOnce(Init) -> B,
    {
        Trans {
            target: f(self.target),
            result: self.result,
        }
    }

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

#[derive(Copy, Clone, Eq, PartialEq, Debug, Display)]
pub struct Unit;

pub type TakeInProgress<Taker> = Trans<Taker, Taker, TerminalTake>;

pub type Take<Taker, Bearer> = Trans<Bundled<Taker, Bearer>, Taker, TerminalTake>;

pub type MakeInProgress<Maker> = Trans<Maker, Maker, Unit>;

pub type Make<Maker, Bearer> = Trans<Bundled<Maker, Bearer>, Maker, Unit>;

impl<T, B> Take<T, B> {
    pub fn added_output(&self) -> OutputAsset<u64>
    where
        T: MarketTaker,
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
        T: MarketTaker,
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
        T: MarketTaker,
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
        T: MarketTaker,
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

    pub fn scale_consumed_budget(&mut self, scale: Ratio<u64>) -> i64
    where
        T: MarketTaker + TakerBehaviour + Copy,
    {
        let consumed_budget = self.consumed_budget();
        match &mut self.result {
            Next::Succ(ref mut next) => {
                let old_val = consumed_budget;
                let new_val = old_val * scale.numer() / scale.denom();
                let delta_consumed_budget = new_val as i64 - old_val as i64;
                let (delta_budget, updated) = next.with_budget_corrected(-delta_consumed_budget);
                let _ = mem::replace(next, updated);
                let delta_consumed_budget = -delta_budget;
                delta_consumed_budget
            }
            Next::Term(ref mut term) => {
                let old_val = consumed_budget;
                let new_val = old_val * scale.numer() / scale.denom();
                let delta_consumed_budget = new_val as i64 - old_val as i64;
                let (delta_budget, updated) = term.with_budget_corrected(-delta_consumed_budget);
                let _ = mem::replace(term, updated);
                let delta_consumed_budget = -delta_budget;
                delta_consumed_budget
            }
        }
    }

    pub fn correct_consumed_budget(&mut self, delta: i64) -> i64
    where
        T: MarketTaker + TakerBehaviour + Copy,
    {
        match &mut self.result {
            Next::Succ(ref mut next) => {
                let (delta_budget, updated) = next.with_budget_corrected(-delta);
                let _ = mem::replace(next, updated);
                let delta_consumed_budget = -delta_budget;
                delta_consumed_budget
            }
            Next::Term(ref mut term) => {
                let (delta_budget, updated) = term.with_budget_corrected(-delta);
                let _ = mem::replace(term, updated);
                let delta_consumed_budget = -delta_budget;
                delta_consumed_budget
            }
        }
    }
}

impl<Maker, B> Make<Maker, B> {
    pub fn trade_side(&self) -> Option<Side>
    where
        Maker: MarketMaker,
    {
        match &self.result {
            Next::Succ(succ) => {
                let AbsoluteReserves {
                    base: succ_reserves_b,
                    quote: succ_reserves_q,
                } = succ.liquidity();
                let AbsoluteReserves {
                    base: init_reserves_b,
                    quote: init_reserved_q,
                } = self.target.0.liquidity();
                if succ_reserves_b < init_reserves_b && succ_reserves_q > init_reserved_q {
                    Some(Side::Bid)
                } else if succ_reserves_b > init_reserves_b && succ_reserves_q < init_reserved_q {
                    Some(Side::Ask)
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    pub fn gain(&self) -> Option<u64>
    where
        Maker: MarketMaker,
    {
        match &self.result {
            Next::Succ(succ) => {
                let AbsoluteReserves {
                    base: succ_reserves_b,
                    quote: succ_reserves_q,
                } = succ.liquidity();
                let AbsoluteReserves {
                    base: init_reserves_b,
                    quote: init_reserves_q,
                } = self.target.0.liquidity();
                succ_reserves_b
                    .checked_sub(init_reserves_b)
                    .or_else(|| succ_reserves_q.checked_sub(init_reserves_q))
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
                let AbsoluteReserves {
                    base: succ_reserves_b,
                    quote: succ_reserves_q,
                } = succ.liquidity();
                let AbsoluteReserves {
                    base: init_reserves_b,
                    quote: init_reserves_q,
                } = self.target.0.liquidity();
                init_reserves_b
                    .checked_sub(succ_reserves_b)
                    .or_else(|| init_reserves_q.checked_sub(succ_reserves_q))
            }
            _ => None,
        }
    }
}

impl<Maker> MakeInProgress<Maker> {
    pub fn trade_side(&self) -> Option<Side>
    where
        Maker: MarketMaker,
    {
        match &self.result {
            Next::Succ(succ) => {
                let AbsoluteReserves {
                    base: succ_reserves_b,
                    quote: succ_reserves_q,
                } = succ.liquidity();
                let AbsoluteReserves {
                    base: init_reserves_b,
                    quote: init_reserved_q,
                } = self.target.liquidity();
                if succ_reserves_b < init_reserves_b && succ_reserves_q > init_reserved_q {
                    Some(Side::Bid)
                } else if succ_reserves_b > init_reserves_b && succ_reserves_q < init_reserved_q {
                    Some(Side::Ask)
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    pub fn gain(&self) -> Option<OnSide<u64>>
    where
        Maker: MarketMaker,
    {
        match &self.result {
            Next::Succ(succ) => {
                let AbsoluteReserves {
                    base: succ_reserves_b,
                    quote: succ_reserves_q,
                } = succ.liquidity();
                let AbsoluteReserves {
                    base: init_reserves_b,
                    quote: init_reserves_q,
                } = self.target.liquidity();
                succ_reserves_b
                    .checked_sub(init_reserves_b)
                    .map(OnSide::Ask)
                    .or_else(|| succ_reserves_q.checked_sub(init_reserves_q).map(OnSide::Bid))
            }
            _ => None,
        }
    }

    pub fn loss(&self) -> Option<OnSide<u64>>
    where
        Maker: MarketMaker,
    {
        match &self.result {
            Next::Succ(succ) => {
                let AbsoluteReserves {
                    base: succ_reserves_b,
                    quote: succ_reserves_q,
                } = succ.liquidity();
                let AbsoluteReserves {
                    base: init_reserves_b,
                    quote: init_reserves_q,
                } = self.target.liquidity();
                init_reserves_b
                    .checked_sub(succ_reserves_b)
                    .map(OnSide::Bid)
                    .or_else(|| init_reserves_q.checked_sub(succ_reserves_q).map(OnSide::Ask))
            }
            _ => None,
        }
    }
}

impl<T> TakeInProgress<T> {
    pub fn removed_input(&self) -> InputAsset<u64>
    where
        T: MarketTaker,
    {
        let remaining_input = match &self.result {
            Next::Succ(next) => next.input(),
            Next::Term(term) => term.remaining_input,
        };
        self.target
            .input()
            .checked_sub(remaining_input)
            .expect("Input cannot increase")
    }

    pub fn added_output(&self) -> OutputAsset<u64>
    where
        T: MarketTaker,
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
    /// Number of distinct makes aggregated into one.
    num_aggregated_makes: usize,
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
            num_aggregated_makes: 0,
        }
    }

    pub fn is_complete(&self) -> bool {
        self.takes.len() > 1 || self.takes.len() == 1 && self.makes.len() > 0
    }

    pub fn needs_rebalancing(&self) -> bool {
        self.num_aggregated_makes > 0
    }

    pub fn execution_units_consumed(&self) -> U
    where
        U: Copy,
    {
        self.execution_units_consumed
    }

    pub fn unsatisfied_fragments(&self) -> Vec<Taker>
    where
        Taker: MarketTaker + Copy,
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

    pub fn next_offered_chunk(&self, taker: &Taker) -> OnSide<u64>
    where
        Taker: MarketTaker,
    {
        let initial_state = self
            .takes
            .get(&taker.stable_id())
            .map(|tr| &tr.target)
            .unwrap_or(taker);
        let initial_chunk = initial_state.input();
        trace!(
            "Initial chunk: {} derived from input: {}",
            initial_chunk,
            initial_state.input()
        );
        let chunk = if initial_chunk > 0 {
            min(initial_chunk, taker.input())
        } else {
            taker.input()
        };
        trace!("Resulted chunk: {}", chunk);
        taker.side().wrap(chunk)
    }

    pub fn add_take(&mut self, take: TakeInProgress<Taker>)
    where
        Taker: MarketTaker<U = U>,
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

    pub fn add_make(&mut self, make: MakeInProgress<Maker>)
    where
        Maker: MarketMaker<U = U>,
        U: AddAssign,
    {
        let sid = make.target.stable_id();
        let aggregate_maker = match self.makes.remove(&sid) {
            None => {
                self.execution_units_consumed += make.target.marginal_cost_hint();
                make
            }
            Some(accumulated_trans) => {
                self.num_aggregated_makes += 1;
                accumulated_trans.combine(make)
            }
        };
        self.makes.insert(sid, aggregate_maker);
    }

    pub fn try_balance(self) -> Option<Self>
    where
        Maker: MakerBalance,
        Taker: MarketTaker + TakerBalance,
    {
        let (mut excess_base, mut excess_quote) = (0u64, 0u64);
        let Self {
            takes,
            makes,
            execution_units_consumed,
            ..
        } = self;
        let mut balanced_makes = vec![];
        for (id, make) in makes {
            let target = make.target;
            match make.result {
                Next::Succ(next) => {
                    let (balanced_next, Excess { base, quote }) = target.balance(next)?;
                    balanced_makes.push((id, MakeInProgress::new(target, Next::Succ(balanced_next))));
                    excess_base += base;
                    excess_quote += quote;
                }
                Next::Term(next) => {
                    balanced_makes.push((id, MakeInProgress::new(target, Next::Term(next))));
                }
            }
        }
        let mut balanced_takes = vec![];
        for (id, take) in takes {
            let excess = match take.target.side() {
                Side::Bid => &mut excess_base,
                Side::Ask => &mut excess_quote,
            };
            if *excess > 0 {
                let result = match take.result {
                    Next::Succ(take) => Next::Succ(take.balance(*excess)),
                    Next::Term(take) => Next::Term(take.balance(*excess)),
                };
                *excess = 0;
                balanced_takes.push((id, TakeInProgress::new(take.target, result)));
            } else {
                balanced_takes.push((id, take));
            }
        }
        if excess_base == 0 && excess_quote == 0 {
            return Some(Self {
                takes: HashMap::from_iter(balanced_takes),
                makes: HashMap::from_iter(balanced_makes),
                execution_units_consumed,
                num_aggregated_makes: 0,
            });
        }
        None
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

impl<T: Display, M: Display> Display for MatchmakingRecipe<T, M> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str("MatchmakingRecipe(")?;
        for i in &self.instructions {
            match i {
                Either::Left(take) => f.write_str(format!("{}, ", take).as_str())?,
                Either::Right(make) => f.write_str(format!("{}, ", make).as_str())?,
            }
        }
        f.write_str(")")
    }
}

pub struct Rebalanced<T>(pub T);

impl<Taker, Maker> MatchmakingRecipe<Taker, Maker>
where
    Taker: Stable,
    Maker: Stable,
{
    pub fn try_from<U>(
        attempt: MatchmakingAttempt<Taker, Maker, U>,
    ) -> Result<Either<Rebalanced<Self>, Self>, Option<Vec<Taker>>>
    where
        Maker: MakerBalance,
        Taker: MarketTaker + TakerBalance + Copy,
    {
        if attempt.is_complete() {
            let needs_rebalancing = attempt.needs_rebalancing();
            let attempt = if needs_rebalancing {
                attempt.try_balance()
            } else {
                Some(attempt)
            };
            if let Some(balanced_attempt) = attempt {
                let unsatisfied_fragments = balanced_attempt.unsatisfied_fragments();
                return if unsatisfied_fragments.is_empty() {
                    let MatchmakingAttempt { takes, makes, .. } = balanced_attempt;
                    let mut instructions = vec![];
                    for take in takes.into_values() {
                        instructions.push(Either::Left(take));
                    }
                    for make in makes.into_values() {
                        instructions.push(Either::Right(make));
                    }
                    let recipe = Self { instructions };
                    Ok(if needs_rebalancing {
                        Either::Left(Rebalanced(recipe))
                    } else {
                        Either::Right(recipe)
                    })
                } else {
                    Err(Some(unsatisfied_fragments))
                };
            }
        }
        Err(None)
    }
}

pub type Execution<T, M, B> = Either<Take<T, B>, Make<M, B>>;

/// Same as [MatchmakingRecipe] but with bearers attached to each target element.
#[derive(Debug, Clone)]
pub struct ExecutionRecipe<Taker, Maker, B>(pub Vec<Execution<Taker, Maker, B>>);

impl<T, M, B> ExecutionRecipe<T, M, B> {
    pub fn link<I, F, V>(
        MatchmakingRecipe { instructions }: MatchmakingRecipe<T, M>,
        link: F,
    ) -> Result<(Self, HashSet<V>), ()>
    where
        V: Hash + Eq,
        T: Stable<StableId = I>,
        M: Stable<StableId = I>,
        F: Fn(I) -> Option<(V, B)>,
    {
        let mut translated_instructions = vec![];
        let mut consumed_versions = vec![];
        for i in instructions {
            match i {
                Either::Left(Trans { target, result }) => {
                    if let Some((ver, bearer)) = link(target.stable_id()) {
                        consumed_versions.push(ver);
                        translated_instructions.push(Either::Left(Trans {
                            target: Bundled(target, bearer),
                            result,
                        }));
                        continue;
                    }
                }
                Either::Right(Trans { target, result }) => {
                    if let Some((ver, bearer)) = link(target.stable_id()) {
                        consumed_versions.push(ver);
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
        Ok((
            Self(translated_instructions),
            HashSet::from_iter(consumed_versions),
        ))
    }
}
