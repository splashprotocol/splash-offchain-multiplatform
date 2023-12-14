use crate::execution_engine::liquidity_book::fragment::{Fragment, OrderState, StateTrans};
use crate::execution_engine::liquidity_book::side::SideM;

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct ExecutionRecipe<Fr, Pl>(pub Vec<TerminalInstruction<Fr, Pl>>);

impl<Fr, Pl> From<IntermediateRecipe<Fr, Pl>> for ExecutionRecipe<Fr, Pl>
where
    Fr: Fragment,
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

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub enum TerminalInstruction<Fr, Pl> {
    Fill(Fill<Fr>),
    Swap(Swap<Pl>),
}

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub struct Fill<Fr> {
    pub target: Fr,
    pub removed_input: u64,
    pub added_output: u64,
}

impl<Fr: Fragment> Fill<Fr> {
    pub fn new(target: Fr, added_output: u64) -> Self {
        Self {
            removed_input: target.input(),
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
    pub fn filled(self) -> (Fill<Fr>, StateTrans<Fr>) {
        (
            Fill {
                target: self.target,
                removed_input: self.target.input(),
                added_output: self.accumulated_output,
            },
            self.target
                .with_updated_liquidity(self.target.input(), self.accumulated_output),
        )
    }
}

impl<Fr: Fragment> From<PartialFill<Fr>> for Fill<Fr> {
    fn from(value: PartialFill<Fr>) -> Self {
        Self {
            removed_input: value.target.input() - value.remaining_input,
            added_output: value.accumulated_output,
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

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub struct Swap<Pl> {
    pub target: Pl,
    pub side: SideM,
    pub input: u64,
    pub output: u64,
}
