use futures::future::Either;

use crate::execution_engine::liquidity_book::fragment::{Fragment, OrderState, StateTrans};
use crate::execution_engine::liquidity_book::side::SideM;

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct ExecutionRecipe<Fr, Pl> {
    pub terminal: Vec<TerminalInstruction<Fr, Pl>>,
    pub remainder: Option<PartialFill<Fr>>,
}

impl<Fr, Pl> ExecutionRecipe<Fr, Pl>
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

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub enum TerminalInstruction<Fr, Pl> {
    Fill(Fill<Fr>),
    Swap(Swap<Pl>),
}

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub struct Fill<Fr> {
    pub target: Fr,
    pub output: u64,
}

impl<Fr> Fill<Fr> {
    pub fn new(target: Fr, output: u64) -> Self {
        Self { target, output }
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
    pub fn into_filled(self) -> (Fill<Fr>, StateTrans<Fr>) {
        (
            Fill {
                target: self.target,
                output: self.accumulated_output,
            },
            self.target
                .with_updated_liquidity(self.target.input(), self.accumulated_output),
        )
    }
}

impl<Fr> From<PartialFill<Fr>> for Fill<Fr> {
    fn from(value: PartialFill<Fr>) -> Self {
        Self {
            target: value.target,
            output: value.accumulated_output,
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

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub struct Swap<Pl> {
    pub target: Pl,
    pub side: SideM,
    pub input: u64,
    pub output: u64,
}
