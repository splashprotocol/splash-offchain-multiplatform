use crate::execution_engine::bundled::Bundled;
use crate::execution_engine::liquidity_book::market_maker::{
    AbsoluteReserves, MakerBehavior, MarketMaker, SpotPrice,
};
use crate::execution_engine::liquidity_book::market_taker::{MarketTaker, TakerBehaviour};
use crate::execution_engine::liquidity_book::side::{OnSide, Side};
use crate::execution_engine::liquidity_book::state::LiquidityBookSize;
use crate::execution_engine::liquidity_book::types::{AbsolutePrice, FeeAsset, InputAsset, OutputAsset};
use algebra_core::monoid::Monoid;
use algebra_core::semigroup::Semigroup;
use bigdecimal::BigDecimal;
use derive_more::{Display, From, Into};
use either::Either;
use log::{trace, warn};
use nonempty::NonEmpty;
use num_bigint::BigInt;
use num_rational::{BigRational, Ratio};
use serde::ser::SerializeStruct;
use serde::{Deserialize, Serialize, Serializer};
use spectrum_offchain::display::display_vec;
use spectrum_offchain::domain::{Has, Stable};
use std::cmp::{max, min};
use std::collections::{HashMap, HashSet, VecDeque};
use std::fmt::Formatter;
use std::hash::Hash;
use std::mem;
use std::ops::AddAssign;
use std::str::FromStr;
use void::Void;

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

    pub fn with_output_added(mut self, added_output: u64) -> Self {
        self.accumulated_output += added_output;
        self
    }

    fn with_fee_charged(mut self, fee: u64) -> Self {
        self.remaining_fee -= fee;
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

pub type FinalTake<Taker> = Final<TakeInProgress<Taker>>;

pub type Take<Taker, Bearer> = Trans<Bundled<Taker, Bearer>, Taker, TerminalTake>;

pub type MakeInProgress<Maker> = Trans<Maker, Maker, Void>;

pub type FinalMake<Maker> = Final<MakeInProgress<Maker>>;

pub type Make<Maker, Bearer> = Trans<Bundled<Maker, Bearer>, Maker, Void>;

#[derive(Debug, Clone)]
pub struct Final<T>(pub T);

#[derive(Debug, Eq, PartialEq)]
pub struct Excess {
    pub base: u64,
    pub quote: u64,
}

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
    pub fn is_zero(&self) -> bool
    where
        Maker: MarketMaker,
    {
        self.trade_side().is_none()
    }

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

    pub fn finalized(self) -> Option<(FinalMake<Maker>, Excess)>
    where
        Maker: MarketMaker + MakerBehavior + Copy,
    {
        let Self { target, result } = self;
        match result {
            Next::Succ(next) => {
                let target_reserves = target.liquidity();
                trace!("R_target {:?}", target_reserves);
                let next_reserves = next.liquidity();
                trace!("R_next {:?}", next_reserves);
                let d_base = next_reserves.base.checked_sub(target_reserves.base);
                if let Some(d_base) = d_base {
                    let trade_input = d_base;
                    let rebalanced = match target.swap(OnSide::Ask(trade_input)) {
                        Next::Succ(maker) => maker,
                        Next::Term(_) => unreachable!(),
                    };
                    let rebalanced_reserves = rebalanced.liquidity();
                    trace!("R_rebalanced in d_base {:?}", rebalanced_reserves);
                    let excess_quote = next_reserves.quote.checked_sub(rebalanced_reserves.quote)?;
                    let delta = Excess {
                        base: 0,
                        quote: excess_quote,
                    };
                    Some((Final(Trans::new(target, Next::Succ(rebalanced))), delta))
                } else {
                    let trade_input = next_reserves.quote.checked_sub(target_reserves.quote)?;
                    let rebalanced = match target.swap(OnSide::Bid(trade_input)) {
                        Next::Succ(maker) => maker,
                        Next::Term(_) => unreachable!(),
                    };
                    let rebalanced_reserves = rebalanced.liquidity();
                    trace!("R_rebalanced in d_quote {:?}", rebalanced_reserves);
                    let excess_base = next_reserves.base.checked_sub(rebalanced_reserves.base)?;
                    let delta = Excess {
                        base: excess_base,
                        quote: 0,
                    };
                    Some((Final(Trans::new(target, Next::Succ(rebalanced))), delta))
                }
            }
            Next::Term(_) => panic!("Maker termination is not supported"),
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

    pub fn finalized(self, excess: u64) -> FinalTake<T>
    where
        T: MarketTaker + TakerBehaviour,
    {
        let removed_input = self.removed_input();
        let Trans { target, result } = self;
        let fee = target.operator_fee(removed_input);
        let budget = target.consumable_budget() as i64;
        let next = match result {
            Next::Succ(next) => {
                let (_, succ) = next
                    .with_output_added(excess)
                    .with_fee_charged(fee)
                    .with_budget_corrected(-budget);
                succ.try_terminate()
            }
            Next::Term(term) => {
                let (_, term) = term
                    .with_output_added(excess)
                    .with_fee_charged(fee)
                    .with_budget_corrected(-budget);
                Next::Term(term)
            }
        };
        Final(Trans::new(target, next))
    }
}

#[derive(Debug, Clone)]
pub struct FinalRecipe<Taker: Stable, Maker: Stable> {
    pub(crate) takes: HashMap<Taker::StableId, FinalTake<Taker>>,
    pub(crate) makes: HashMap<Maker::StableId, FinalMake<Maker>>,
    pub(crate) ordering: Vec<Taker::StableId>,
}

impl<T: Stable, M: Stable> FinalRecipe<T, M> {
    pub fn unsatisfied_fragments(&self) -> Vec<T>
    where
        T: MarketTaker + Copy,
    {
        self.takes
            .iter()
            .filter_map(|(_, Final(apply))| {
                let target = apply.target;
                if apply.added_output() < target.min_marginal_output() {
                    Some(target)
                } else {
                    None
                }
            })
            .collect()
    }
}

#[derive(Debug, Clone)]
pub struct MatchmakingAttempt<Taker: Stable, Maker: Stable, U> {
    takes: HashMap<Taker::StableId, TakeInProgress<Taker>>,
    makes: HashMap<Maker::StableId, MakeInProgress<Maker>>,
    ordering: Vec<Taker::StableId>,
    execution_units_consumed: U,
    /// Number of distinct makes aggregated into one.
    num_aggregated_makes: usize,
}

impl<T: Stable + Display, M: Stable + Display, U> Display for MatchmakingAttempt<T, M, U> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(
            format!(
                "MatchmakingAttempt(takes: {}, makes: {}, num_aggregated_makes: {})",
                display_vec(&self.takes.values().collect()),
                display_vec(&self.makes.values().collect()),
                self.num_aggregated_makes
            )
            .as_str(),
        )
    }
}

impl<Taker: Stable, Maker: Stable, U> MatchmakingAttempt<Taker, Maker, U> {
    pub fn empty() -> Self
    where
        U: Monoid,
    {
        Self {
            takes: HashMap::new(),
            makes: HashMap::new(),
            ordering: Vec::new(),
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

    pub fn num_takes(&self) -> usize {
        self.takes.len()
    }

    pub fn execution_units_consumed(&self) -> U
    where
        U: Copy,
    {
        self.execution_units_consumed
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
                self.ordering.push(sid);
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

    pub fn finalized(self) -> Option<FinalRecipe<Taker, Maker>>
    where
        Maker: MarketMaker + MakerBehavior + Copy,
        Taker: MarketTaker + TakerBehaviour,
    {
        let (mut excess_base, mut excess_quote) = (0u64, 0u64);
        let Self {
            takes,
            makes,
            ordering,
            ..
        } = self;
        let mut balanced_makes = vec![];
        for (id, make) in makes {
            let (final_make, Excess { base, quote }) = make.finalized()?;
            balanced_makes.push((id, final_make));
            excess_base += base;
            excess_quote += quote;
        }
        let mut balanced_takes = vec![];
        for (id, take) in takes {
            let excess = match take.target.side() {
                Side::Bid => &mut excess_base,
                Side::Ask => &mut excess_quote,
            };
            balanced_takes.push((id, take.finalized(*excess)));
            *excess = 0;
        }
        if excess_base == 0 && excess_quote == 0 {
            return Some(FinalRecipe {
                takes: HashMap::from_iter(balanced_takes),
                makes: HashMap::from_iter(balanced_makes),
                ordering,
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

const BASE_TAKE_TO_MAKE_RATIO: Ratio<usize> = Ratio::new_raw(1, 2);

#[derive(Debug, Copy, Clone, PartialEq, Eq, From, Into, serde::Deserialize)]
pub struct BaseStepBudget(u64);

/// An exception indicating the recipe turned out to be too expensive for available budget.
pub struct DowngradeNeeded;

/// Takers that require deeper execution than current recipe provides.
pub type DissatisfiedTakers<Taker> = Vec<Taker>;

impl<Taker, Maker> MatchmakingRecipe<Taker, Maker>
where
    Taker: Stable,
    Maker: Stable,
{
    pub fn try_from<U, C>(
        attempt: MatchmakingAttempt<Taker, Maker, U>,
        cx: C,
    ) -> Result<Self, Option<Either<DissatisfiedTakers<Taker>, DowngradeNeeded>>>
    where
        Maker: MarketMaker + MakerBehavior + Copy,
        Taker: MarketTaker + TakerBehaviour + Copy,
        C: Has<BaseStepBudget>,
    {
        if attempt.is_complete() {
            if let Some(final_recipe) = attempt.finalized() {
                let unsatisfied_fragments = final_recipe.unsatisfied_fragments();
                return if unsatisfied_fragments.is_empty() {
                    let complexity_ok = Self::check_recipe_complexity(&final_recipe, cx);
                    let FinalRecipe {
                        takes,
                        makes,
                        ordering,
                    } = final_recipe;
                    let mut ordering = ordering
                        .into_iter()
                        .enumerate()
                        .map(|(i, id)| (id, i))
                        .collect::<HashMap<_, _>>();
                    let mut takes = takes.into_values().collect::<Vec<_>>();
                    takes.sort_by_cached_key(|x| {
                        ordering
                            .remove(&x.0.target.stable_id())
                            .expect("Recipe is inconsistent")
                    });
                    let mut instructions = vec![];
                    for Final(take) in takes {
                        instructions.push(Either::Left(take));
                    }
                    let mut has_zero_makes = false;
                    for Final(make) in makes.into_values() {
                        has_zero_makes = has_zero_makes || make.is_zero();
                        instructions.push(Either::Right(make));
                    }
                    if !has_zero_makes {
                        if complexity_ok {
                            Ok(Self { instructions })
                        } else {
                            Err(Some(Either::Right(DowngradeNeeded)))
                        }
                    } else {
                        warn!("Zero make appeared in recipe");
                        Err(None)
                    }
                } else {
                    Err(Some(Either::Left(unsatisfied_fragments)))
                };
            }
        }
        Err(None)
    }

    fn check_recipe_complexity<C>(recipe: &FinalRecipe<Taker, Maker>, cx: C) -> bool
    where
        Taker: MarketTaker,
        C: Has<BaseStepBudget>,
    {
        let num_takes = recipe.takes.len();
        let num_makes = recipe.makes.len();
        if num_makes > 0 {
            let takes_to_makes = Ratio::new_raw(num_takes, num_makes);
            let base_budget = max(<u64>::from(cx.get()), 1);
            let total_budget = recipe
                .takes
                .values()
                .fold(0, |acc, Final(take)| acc + take.target.consumable_budget());
            let budget_ratio = Ratio::new_raw(total_budget as usize, base_budget as usize);
            let takes_to_makes_ratio = takes_to_makes * budget_ratio.reduced();
            return takes_to_makes_ratio >= BASE_TAKE_TO_MAKE_RATIO;
        }
        true
    }
}

pub type Execution<T, M, B> = Either<Take<T, B>, Make<M, B>>;

type Orphans<T, M> = NonEmpty<Either<T, M>>;

/// Same as [MatchmakingRecipe] but with bearers attached to each target element.
/// Returns a collection of orphaned targets that failed to link in case of failure.
#[derive(Debug, Clone)]
pub struct ExecutionRecipe<Taker, Maker, B>(pub Vec<Execution<Taker, Maker, B>>);

impl<T, M, B> ExecutionRecipe<T, M, B> {
    /// Link the recipe to corresponding bearers.
    /// Returns [ExecutionRecipe] along with consumed bearers in case of success.
    /// If linking fails, a set of orphaned entities is returned.
    pub fn link<I, F, V>(
        MatchmakingRecipe { instructions }: MatchmakingRecipe<T, M>,
        link: F,
    ) -> Result<(Self, HashSet<V>), Orphans<T, M>>
    where
        V: Hash + Eq,
        T: Stable<StableId = I>,
        M: Stable<StableId = I>,
        F: Fn(I) -> Option<(V, B)>,
    {
        let mut translated_instructions = vec![];
        let mut consumed_versions = vec![];
        let mut orphaned_targets = vec![];
        for i in instructions {
            match i {
                Either::Left(Trans { target, result }) => {
                    if let Some((ver, bearer)) = link(target.stable_id()) {
                        consumed_versions.push(ver);
                        translated_instructions.push(Either::Left(Trans {
                            target: Bundled(target, bearer),
                            result,
                        }));
                    } else {
                        orphaned_targets.push(Either::Left(target));
                    }
                }
                Either::Right(Trans { target, result }) => {
                    if let Some((ver, bearer)) = link(target.stable_id()) {
                        consumed_versions.push(ver);
                        translated_instructions.push(Either::Right(Trans {
                            target: Bundled(target, bearer),
                            result,
                        }));
                    } else {
                        orphaned_targets.push(Either::Right(target));
                    }
                }
            }
        }
        if let Some(orphans) = NonEmpty::collect(orphaned_targets) {
            return Err(orphans);
        }
        Ok((
            Self(translated_instructions),
            HashSet::from_iter(consumed_versions),
        ))
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum ExecutionEvent {
    SpotPrice(SpotPrice),
    SpotPriceNotAvailable,
    LiquidityBookSizePreAttempt(LiquidityBookSize),
    LiquidityBookSizePostAttempt(LiquidityBookSize),
}

#[cfg(test)]
mod tests {
    use crate::execution_engine::liquidity_book::core::{
        BaseStepBudget, Final, FinalRecipe, MatchmakingRecipe, Next, TerminalTake, Trans,
    };
    use crate::execution_engine::liquidity_book::market_taker::MarketTaker;
    use crate::execution_engine::liquidity_book::side::Side;
    use crate::execution_engine::liquidity_book::time::TimeBounds;
    use crate::execution_engine::liquidity_book::types::{AbsolutePrice, FeeAsset, InputAsset, OutputAsset};
    use spectrum_offchain::domain::{Has, Stable};
    use std::collections::HashMap;
    use type_equalities::IsEqual;

    #[test]
    fn recipe_complexity_estimation_ok() {
        let recipe = FinalRecipe::<Taker, Maker> {
            takes: HashMap::from([(
                0usize,
                Final(Trans::new(
                    Taker {
                        id: 0,
                        budget: 1100000,
                    },
                    Next::Term(TerminalTake {
                        remaining_input: 0,
                        accumulated_output: 0,
                        remaining_budget: 0,
                        remaining_fee: 0,
                    }),
                )),
            )]),
            makes: HashMap::from([
                (
                    1usize,
                    Final(Trans::new(Maker { id: 1 }, Next::Succ(Maker { id: 1 }))),
                ),
                (2, Final(Trans::new(Maker { id: 2 }, Next::Succ(Maker { id: 2 })))),
            ]),
            ordering: vec![],
        };
        let ok = MatchmakingRecipe::check_recipe_complexity(
            &recipe,
            Cx {
                base_step_budget: 1100000.into(),
            },
        );
        assert!(ok)
    }

    #[test]
    fn recipe_complexity_estimation_double_budget_ok() {
        let recipe = FinalRecipe::<Taker, Maker> {
            takes: HashMap::from([(
                0usize,
                Final(Trans::new(
                    Taker {
                        id: 0,
                        budget: 1100000 * 2,
                    },
                    Next::Term(TerminalTake {
                        remaining_input: 0,
                        accumulated_output: 0,
                        remaining_budget: 0,
                        remaining_fee: 0,
                    }),
                )),
            )]),
            makes: HashMap::from([
                (1, Final(Trans::new(Maker { id: 1 }, Next::Succ(Maker { id: 1 })))),
                (2, Final(Trans::new(Maker { id: 2 }, Next::Succ(Maker { id: 2 })))),
                (3, Final(Trans::new(Maker { id: 3 }, Next::Succ(Maker { id: 3 })))),
            ]),
            ordering: vec![],
        };
        let ok = MatchmakingRecipe::check_recipe_complexity(
            &recipe,
            Cx {
                base_step_budget: 1100000.into(),
            },
        );
        assert!(ok)
    }

    #[test]
    fn recipe_complexity_estimation_mult_takers_ok() {
        let recipe = FinalRecipe::<Taker, Maker> {
            takes: HashMap::from([
                (
                    0,
                    Final(Trans::new(
                        Taker {
                            id: 0,
                            budget: 1100000,
                        },
                        Next::Term(TerminalTake {
                            remaining_input: 0,
                            accumulated_output: 0,
                            remaining_budget: 0,
                            remaining_fee: 0,
                        }),
                    )),
                ),
                (
                    1,
                    Final(Trans::new(
                        Taker {
                            id: 1,
                            budget: 1100000,
                        },
                        Next::Term(TerminalTake {
                            remaining_input: 0,
                            accumulated_output: 0,
                            remaining_budget: 0,
                            remaining_fee: 0,
                        }),
                    )),
                ),
            ]),
            makes: HashMap::from([
                (1, Final(Trans::new(Maker { id: 1 }, Next::Succ(Maker { id: 1 })))),
                (2, Final(Trans::new(Maker { id: 2 }, Next::Succ(Maker { id: 2 })))),
                (3, Final(Trans::new(Maker { id: 3 }, Next::Succ(Maker { id: 3 })))),
            ]),
            ordering: vec![],
        };
        let ok = MatchmakingRecipe::check_recipe_complexity(
            &recipe,
            Cx {
                base_step_budget: 1100000.into(),
            },
        );
        assert!(ok)
    }

    #[test]
    fn recipe_complexity_estimation_not_ok() {
        let recipe = FinalRecipe::<Taker, Maker> {
            takes: HashMap::from([(
                0usize,
                Final(Trans::new(
                    Taker {
                        id: 0,
                        budget: 1100000,
                    },
                    Next::Term(TerminalTake {
                        remaining_input: 0,
                        accumulated_output: 0,
                        remaining_budget: 0,
                        remaining_fee: 0,
                    }),
                )),
            )]),
            makes: HashMap::from([
                (1, Final(Trans::new(Maker { id: 1 }, Next::Succ(Maker { id: 1 })))),
                (2, Final(Trans::new(Maker { id: 2 }, Next::Succ(Maker { id: 2 })))),
                (3, Final(Trans::new(Maker { id: 3 }, Next::Succ(Maker { id: 3 })))),
            ]),
            ordering: vec![],
        };
        let ok = MatchmakingRecipe::check_recipe_complexity(
            &recipe,
            Cx {
                base_step_budget: 1100000.into(),
            },
        );
        assert!(!ok)
    }

    struct Maker {
        id: usize,
    }

    impl Stable for Maker {
        type StableId = usize;
        fn stable_id(&self) -> Self::StableId {
            self.id
        }
        fn is_quasi_permanent(&self) -> bool {
            false
        }
    }

    struct Taker {
        id: usize,
        budget: u64,
    }

    impl Stable for Taker {
        type StableId = usize;
        fn stable_id(&self) -> Self::StableId {
            self.id
        }
        fn is_quasi_permanent(&self) -> bool {
            false
        }
    }

    impl MarketTaker for Taker {
        type U = ();

        fn side(&self) -> Side {
            todo!()
        }

        fn input(&self) -> InputAsset<u64> {
            todo!()
        }

        fn output(&self) -> OutputAsset<u64> {
            todo!()
        }

        fn price(&self) -> AbsolutePrice {
            todo!()
        }

        fn operator_fee(&self, input_consumed: InputAsset<u64>) -> FeeAsset<u64> {
            todo!()
        }

        fn fee(&self) -> FeeAsset<u64> {
            todo!()
        }

        fn budget(&self) -> FeeAsset<u64> {
            self.budget
        }

        fn consumable_budget(&self) -> FeeAsset<u64> {
            self.budget
        }

        fn marginal_cost_hint(&self) -> Self::U {
            todo!()
        }

        fn min_marginal_output(&self) -> OutputAsset<u64> {
            todo!()
        }

        fn time_bounds(&self) -> TimeBounds<u64> {
            todo!()
        }
    }

    struct Cx {
        base_step_budget: BaseStepBudget,
    }

    impl Has<BaseStepBudget> for Cx {
        fn select<U: IsEqual<BaseStepBudget>>(&self) -> BaseStepBudget {
            self.base_step_budget
        }
    }
}
