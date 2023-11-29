use futures::future::Either;

use crate::fragment::Fragment;
use crate::side::{Side, SideMarker};

#[derive(Debug, Clone)]
pub struct ExecutionRecipe<Fr, Pl> {
    pub terminal: Vec<TerminalInstruction<Fr, Pl>>,
    pub remainder: Option<Side<PartialFill<Fr>>>,
}

impl<Fr, Pl> ExecutionRecipe<Fr, Pl>
where
    Fr: Fragment,
{
    pub fn new(fr: Side<Fr>) -> Self {
        Self {
            terminal: Vec::new(),
            remainder: Some(fr.map(PartialFill::new)),
        }
    }
    pub fn push(&mut self, instruction: TerminalInstruction<Fr, Pl>) {
        self.terminal.push(instruction)
    }
    pub fn terminate(&mut self, instruction: TerminalInstruction<Fr, Pl>) {
        self.push(instruction);
        self.remainder = None;
    }
    pub fn set_remainder(&mut self, remainder: Side<PartialFill<Fr>>) {
        self.remainder = Some(remainder);
    }
    pub fn is_complete(&self) -> bool {
        let terminal_fragments = self.terminal.len();
        terminal_fragments % 2 == 0 || (terminal_fragments > 0 && self.remainder.is_some())
    }
    pub fn disassemble(self) -> Vec<Either<Side<Fr>, Pl>> {
        let mut acc = Vec::new();
        for i in self.terminal {
            match i {
                TerminalInstruction::Fill(fill) => acc.push(Either::Left(fill.map(|f| f.target))),
                TerminalInstruction::Swap(swap) => acc.push(Either::Right(swap.target)),
            }
        }
        if let Some(partial) = self.remainder {
            acc.push(Either::Left(partial.map(|f| f.target)));
        }
        acc
    }
}

#[derive(Debug, Copy, Clone)]
pub enum TerminalInstruction<Fr, Pl> {
    Fill(Side<Fill<Fr>>),
    Swap(Swap<Pl>),
}

#[derive(Debug, Copy, Clone)]
pub struct Fill<Fr> {
    pub target: Fr,
    pub output: u64,
}

impl<Fr> Fill<Fr> {
    pub fn new(target: Fr, output: u64) -> Self {
        Self { target, output }
    }
}

#[derive(Debug, Copy, Clone)]
pub struct PartialFill<Fr> {
    pub target: Fr,
    pub remaining_input: u64,
    pub accumulated_output: u64,
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

#[derive(Debug, Copy, Clone)]
pub struct Swap<Pl> {
    pub target: Pl,
    pub side: SideMarker,
    pub input: u64,
    pub output: u64,
}
