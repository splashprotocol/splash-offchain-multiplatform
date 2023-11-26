use crate::liquidity_book::fragment::Fragment;
use crate::liquidity_book::side::{Side, SideMarker};

#[derive(Debug, Clone)]
pub struct ExecutionRecipe<Fr, Pl> {
    pub terminal: Vec<TerminalInstruction<Fr, Pl>>,
    pub remainder: Option<Side<PartialFill<Fr>>>,
}

impl<T, Pl> ExecutionRecipe<Fragment<T>, Pl> {
    pub fn new(fr: Side<Fragment<T>>) -> Self {
        Self {
            terminal: Vec::new(),
            remainder: Some(fr.map(PartialFill::new)),
        }
    }
    pub fn push(&mut self, instruction: TerminalInstruction<Fragment<T>, Pl>) {
        self.terminal.push(instruction)
    }
    pub fn terminate(&mut self, instruction: TerminalInstruction<Fragment<T>, Pl>) {
        self.push(instruction);
        self.remainder = None;
    }
    pub fn set_remainder(&mut self, remainder: Side<PartialFill<Fragment<T>>>) {
        self.remainder = Some(remainder);
    }
}

#[derive(Debug, Copy, Clone)]
pub enum TerminalInstruction<Fr, Pl> {
    Fill(Fill<Fr>),
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

impl<T> PartialFill<Fragment<T>> {
    pub fn new(fr: Fragment<T>) -> Self {
        Self {
            remaining_input: fr.input,
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
