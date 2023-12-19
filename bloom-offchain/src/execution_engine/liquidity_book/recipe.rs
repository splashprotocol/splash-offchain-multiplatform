use crate::execution_engine::bundled::Bundled;
use crate::execution_engine::liquidity_book::fragment::{Fragment, OrderState, StateTrans};
use crate::execution_engine::liquidity_book::side::SideM;

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct LinkedExecutionRecipe<Fr, Pl, Src>(pub Vec<LinkedTerminalInstruction<Fr, Pl, Src>>);

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
            remainder: Some(PartialFill::new(fr)),
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
pub enum LinkedTerminalInstruction<Fr, Pl, Src> {
    Fill(LinkedFill<Fr, Src>),
    Swap(LinkedSwap<Pl, Src>),
}

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub enum TerminalInstruction<Fr, Pl> {
    Fill(Fill<Fr>),
    Swap(Swap<Pl>),
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct LinkedFill<Fr, Src> {
    pub target: Bundled<Fr, Src>,
    pub transition: StateTrans<Fr>,
    pub removed_input: u64,
    pub added_output: u64,
}

impl<Fr, Src> LinkedFill<Fr, Src> {
    pub fn from_fill(fill: Fill<Fr>, target_src: Src) -> Self {
        Self {
            target: Bundled(fill.target, target_src),
            transition: fill.transition,
            removed_input: fill.removed_input,
            added_output: fill.added_output,
        }
    }
}

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub struct Fill<Fr> {
    pub target: Fr,
    pub transition: StateTrans<Fr>,
    pub removed_input: u64,
    pub added_output: u64,
}

impl<Fr: Fragment> Fill<Fr> {
    pub fn new(target: Fr, transition: StateTrans<Fr>, added_output: u64) -> Self {
        Self {
            removed_input: target.input(),
            transition,
            target,
            added_output,
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
        Fill {
            target: self.target,
            transition: self
                .target
                .with_updated_liquidity(self.target.input(), self.accumulated_output),
            removed_input: self.target.input(),
            added_output: self.accumulated_output,
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
        Self {
            removed_input: removed,
            transition: value.target.with_updated_liquidity(removed, added),
            added_output: added,
            target: value.target,
        }
    }
}

impl<Fr> PartialFill<Fr>
where
    Fr: Fragment,
{
    pub fn new(fr: Fr) -> Self {
        Self {
            remaining_input: fr.input(),
            target: fr,
            accumulated_output: 0,
        }
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct LinkedSwap<Pl, Src> {
    pub target: Bundled<Pl, Src>,
    pub transition: Pl,
    pub side: SideM,
    pub input: u64,
    pub output: u64,
}

impl<Pl, Src> LinkedSwap<Pl, Src> {
    pub fn from_swap(swap: Swap<Pl>, target_src: Src) -> Self {
        Self {
            target: Bundled(swap.target, target_src),
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
