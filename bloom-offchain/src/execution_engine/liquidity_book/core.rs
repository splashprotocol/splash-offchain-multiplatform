use std::cmp::min;
use std::collections::HashMap;

use bounded_integer::BoundedU64;
use either::Either;

use algebra_core::monoid::Monoid;
use spectrum_offchain::data::Stable;

use crate::execution_engine::liquidity_book::fragment::Fragment;
use crate::execution_engine::liquidity_book::side::{Side, SideM};
use crate::execution_engine::liquidity_book::types::{FeeAsset, InputAsset, OutputAsset};

pub type MatchmakingStep = BoundedU64<0, 100>;

/// Usage of liquidity from market maker.
/// take(P, M_1 + M_2 + ... + M_n) = take(P, M_1) |> take(_, M_2) |> ... take(_, M_n)
#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub struct Make {
    pub side: SideM,
    pub input: InputAsset<u64>,
    pub output: OutputAsset<u64>,
}

/// Taking liquidity from market.
#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub struct Take {
    /// Input asset removed as a result of this transaction.
    pub removed_input: InputAsset<u64>,
    /// Output asset added as a result of this transaction.
    pub added_output: OutputAsset<u64>,
    /// Overall execution budget used.
    pub budget_used: FeeAsset<u64>,
    /// Execution fee charged.
    pub fee_used: FeeAsset<u64>,
}

impl Take {
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
pub enum Next<T> {
    /// Successive state is available.
    Succ(T),
    /// Terminal state.
    Term,
}

#[derive(Debug, Copy, Clone)]
pub struct TryApply<Action, Subject> {
    pub action: Action,
    pub target: Subject,
    pub result: Next<Subject>,
}

#[derive(Debug, Copy, Clone)]
pub struct TakeInProgress<Taker> {
    pub action: Take,
    pub target: Taker,
    pub result: Option<Next<Taker>>,
}

impl<Taker> TakeInProgress<Taker> {
    pub fn new(target: Taker) -> Self {
        Self {
            action: Take::new(),
            target,
            result: None,
        }
    }

    pub fn remaining_amount_offered(&self) -> InputAsset<u64> where Taker: Fragment {
        self.target.input() - self.action.removed_input
    }

    pub fn next_chunk_offered(&self, step: MatchmakingStep) -> Side<InputAsset<u64>> where Taker: Fragment {
        let chunk = self.target.input() * step.get() / 100;
        self.target.side().wrap(if chunk > 0 {
            min(self.target.input() * step.get() / 100, self.remaining_amount_offered())
        } else {
            self.remaining_amount_offered()
        })
    }
}

#[derive(Debug, Clone)]
pub struct MatchmakingAttempt<Taker: Stable, Maker: Stable, U> {
    terminal_takes: HashMap<Taker::StableId, TryApply<Take, Taker>>,
    makes: HashMap<Maker::StableId, TryApply<Make, Maker>>,
    remainder: Option<TakeInProgress<Taker>>,
    execution_units_consumed: U,
}

impl<Taker: Stable, Maker: Stable, U> MatchmakingAttempt<Taker, Maker, U> {
    pub fn empty() -> Self
        where
            U: Monoid,
    {
        Self {
            terminal_takes: HashMap::new(),
            makes: HashMap::new(),
            remainder: None,
            execution_units_consumed: U::empty(),
        }
    }

    pub fn remainder(&self) -> &Option<TakeInProgress<Taker>> {
        &self.remainder
    }

    pub fn set_remainder(&mut self, take: TakeInProgress<Taker>) {
        self.remainder = Some(take);
    }

    pub fn execution_units_consumed(&self) -> U
        where
            U: Copy,
    {
        self.execution_units_consumed
    }

    pub fn is_complete(&self) -> bool {
        let num_takes_terminated = self.terminal_takes.len();
        num_takes_terminated >= 2 || (num_takes_terminated > 0 && self.remainder.is_some())
    }

    pub fn unsatisfied_fragments(&self) -> Vec<Taker>
        where
            Taker: Fragment + Copy,
    {
        let not_ok_terminal_takes = self.terminal_takes.iter().filter_map(|(_, apply)| {
            let target = apply.target;
            if apply.action.added_output < target.min_marginal_output() {
                Some(target)
            } else {
                None
            }
        });
        let not_ok_non_terminal_fills = self
            .remainder
            .as_ref()
            .filter(|take| take.action.added_output < take.target.min_marginal_output())
            .map(|fill| fill.target);
        not_ok_terminal_takes.chain(not_ok_non_terminal_fills).collect()
    }
}

#[derive(Debug, Copy, Clone)]
pub struct Applied<Action, Subject: Stable> {
    pub action: Action,
    pub target: Subject::StableId,
    pub result: Next<Subject>,
}

#[derive(Debug, Clone)]
pub struct MatchmakingRecipe<Taker: Stable, Maker: Stable> {
    instructions: Vec<Either<Applied<Take, Taker>, Applied<Make, Maker>>>,
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
                let MatchmakingAttempt {
                    terminal_takes,
                    makes,
                    remainder,
                    ..
                } = attempt;
                let mut instructions = Vec::new();
                for (taker_id, TryApply { action, result, .. }) in terminal_takes {
                    instructions.push(Either::Left(Applied {
                        action,
                        result,
                        target: taker_id,
                    }));
                }
                for (maker_id, TryApply { action, result, .. }) in makes {
                    instructions.push(Either::Right(Applied {
                        action,
                        result,
                        target: maker_id,
                    }));
                }
                if let Some(TakeInProgress {
                                action,
                                result: Some(result),
                                target,
                            }) = remainder
                {
                    instructions.push(Either::Left(Applied {
                        action,
                        result,
                        target: target.stable_id(),
                    }));
                }
                return Ok(Self { instructions });
            } else {
                return Err(Some(unsatisfied_fragments));
            }
        }
        Err(None)
    }
}
