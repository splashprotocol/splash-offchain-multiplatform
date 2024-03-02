use crate::execution_engine::bundled::Bundled;
use crate::execution_engine::liquidity_book::fragment::{Fragment, OrderState, StateTrans};
use crate::execution_engine::liquidity_book::side::SideM;
use crate::execution_engine::liquidity_book::types::{FeeAsset, InputAsset, OutputAsset};

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct LinkedExecutionRecipe<Fr, Pl, Src, Ver>(pub Vec<LinkedTerminalInstruction<Fr, Pl, Src, Ver>>);

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct ExecutionRecipe<Fr, Pl>(pub Vec<TerminalInstruction<Fr, Pl>>);

impl<Fr, Pl> From<IntermediateRecipe<Fr, Pl>> for ExecutionRecipe<Fr, Pl>
where
    Fr: Fragment + OrderState + Copy,
{
    fn from(
        IntermediateRecipe {
            mut terminal,
            remainder,
        }: IntermediateRecipe<Fr, Pl>,
    ) -> Self {
        if let Some(rem) = remainder {
            terminal.push(TerminalInstruction::Fill(rem.into()));
        }
        Self(terminal)
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct IntermediateRecipe<Fr, Pl> {
    pub terminal: Vec<TerminalInstruction<Fr, Pl>>,
    pub remainder: Option<PartialFill<Fr>>,
}

impl<Fr, Pl> IntermediateRecipe<Fr, Pl>
where
    Fr: Fragment,
{
    pub fn new(fr: Fr) -> Self {
        Self {
            terminal: Vec::new(),
            remainder: Some(PartialFill::empty(fr)),
        }
    }

    pub fn push(&mut self, instruction: TerminalInstruction<Fr, Pl>) {
        self.terminal.push(instruction)
    }

    pub fn terminate(&mut self, instruction: TerminalInstruction<Fr, Pl>) {
        self.push(instruction);
        self.remainder = None;
    }

    pub fn set_remainder(&mut self, remainder: PartialFill<Fr>) {
        self.remainder = Some(remainder);
    }

    pub fn is_complete(&self) -> bool {
        let terminal_fragments = self.terminal.len();
        terminal_fragments >= 2 || (terminal_fragments > 0 && self.remainder.is_some())
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub enum LinkedTerminalInstruction<Fr, Pl, Src, Ver> {
    Fill(LinkedFill<Fr, Src, Ver>),
    Swap(LinkedSwap<Pl, Src, Ver>),
}

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub enum TerminalInstruction<Fr, Pl> {
    Fill(Fill<Fr>),
    Swap(Swap<Pl>),
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct LinkedFill<Fr, Src, Ver> {
    pub target_fr: Bundled<Fr, Src, Ver>,
    pub next_fr: StateTrans<Fr>,
    pub removed_input: InputAsset<u64>,
    pub added_output: OutputAsset<u64>,
    pub budget_used: FeeAsset<u64>,
    pub fee_used: FeeAsset<u64>,
}

impl<Fr, Src, Ver> LinkedFill<Fr, Src, Ver> {
    pub fn from_fill(fill: Fill<Fr>, target_src: Src) -> Self {
        Self {
            target_fr: Bundled::new(fill.target_fr, target_src),
            next_fr: fill.next_fr,
            removed_input: fill.removed_input,
            added_output: fill.added_output,
            budget_used: fill.budget_used,
            fee_used: fill.fee_used,
        }
    }
}

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub struct Fill<Fr> {
    pub target_fr: Fr,
    /// Next fragment [Fr] resulted from this transaction.
    pub next_fr: StateTrans<Fr>,
    /// Input asset removed as a result of this transaction.
    pub removed_input: InputAsset<u64>,
    /// Output asset added as a result of this transaction.
    pub added_output: OutputAsset<u64>,
    /// Overall execution budget used.
    pub budget_used: FeeAsset<u64>,
    /// Execution fee charged.
    pub fee_used: FeeAsset<u64>,
}

impl<Fr: Fragment> Fill<Fr> {
    pub fn new(
        target: Fr,
        transition: StateTrans<Fr>,
        added_output: OutputAsset<u64>,
        budget_used: FeeAsset<u64>,
        fee_used: FeeAsset<u64>,
    ) -> Self {
        Self {
            removed_input: target.input(),
            budget_used,
            next_fr: transition,
            target_fr: target,
            added_output,
            fee_used,
        }
    }
}

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub struct PartialFill<Fr> {
    pub target: Fr,
    pub remaining_input: u64,
    pub accumulated_output: u64,
}

impl<Fr> PartialFill<Fr>
where
    Fr: Fragment + OrderState + Copy,
{
    /// Force fill target fragment.
    /// Does not guarantee that the fragment is actually fully satisfied.
    pub fn filled_unsafe(self) -> Fill<Fr> {
        let (tx, budget_used, fee_used) = self
            .target
            .with_applied_swap(self.target.input(), self.accumulated_output);
        Fill {
            target_fr: self.target,
            next_fr: tx,
            removed_input: self.target.input(),
            added_output: self.accumulated_output,
            budget_used,
            fee_used,
        }
    }
}

impl<Fr: Fragment> From<PartialFill<Fr>> for Fill<Fr>
where
    Fr: OrderState + Copy,
{
    fn from(value: PartialFill<Fr>) -> Self {
        let removed = value.target.input() - value.remaining_input;
        let added = value.accumulated_output;
        let (transition, budget_used, fee_used) = value.target.with_applied_swap(removed, added);
        Self {
            removed_input: removed,
            next_fr: transition,
            added_output: added,
            target_fr: value.target,
            budget_used,
            fee_used,
        }
    }
}

impl<Fr> PartialFill<Fr>
where
    Fr: Fragment,
{
    pub fn new(target: Fr, remaining_input: u64, added_output: u64) -> Self {
        Self {
            target,
            remaining_input,
            accumulated_output: added_output,
        }
    }

    pub fn empty(fr: Fr) -> Self {
        Self {
            remaining_input: fr.input(),
            target: fr,
            accumulated_output: 0,
        }
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct LinkedSwap<Pl, Src, Ver> {
    pub target: Bundled<Pl, Src, Ver>,
    pub transition: Pl,
    pub side: SideM,
    pub input: u64,
    pub output: u64,
}

impl<Pl, Src, Ver> LinkedSwap<Pl, Src, Ver> {
    pub fn from_swap(swap: Swap<Pl>, target_src: Src) -> Self {
        Self {
            target: Bundled::new(swap.target, target_src),
            transition: swap.transition,
            side: swap.side,
            input: swap.input,
            output: swap.output,
        }
    }
}

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub struct Swap<Pl> {
    pub target: Pl,
    pub transition: Pl,
    pub side: SideM,
    pub input: u64,
    pub output: u64,
}
