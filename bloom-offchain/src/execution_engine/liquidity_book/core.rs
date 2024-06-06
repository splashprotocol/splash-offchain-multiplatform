use std::collections::HashMap;

use spectrum_offchain::data::Stable;

use crate::execution_engine::liquidity_book::side::SideM;
use crate::execution_engine::liquidity_book::types::{FeeAsset, InputAsset, OutputAsset};

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub struct Take {
    pub side: SideM,
    pub input: InputAsset<u64>,
    pub output: OutputAsset<u64>,
}

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub struct Make {
    /// Input asset removed as a result of this transaction.
    pub removed_input: InputAsset<u64>,
    /// Output asset added as a result of this transaction.
    pub added_output: OutputAsset<u64>,
    /// Overall execution budget used.
    pub budget_used: FeeAsset<u64>,
    /// Execution fee charged.
    pub fee_used: FeeAsset<u64>,
}

impl Make {
    pub fn new() -> Self {
        Self {
            removed_input: 0,
            added_output: 0,
            budget_used: 0,
            fee_used: 0,
        }
    }
}

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub struct MakeInProgress<Taker> {
    pub initial: Taker,
    pub make: Make,
}

impl<Taker> MakeInProgress<Taker> {
    pub fn new(initial_taker: Taker) -> Self {
        Self {
            initial: initial_taker,
            make: Make::new(),
        }
    }
}

#[derive(Debug, Copy, Clone)]
pub enum Next<T> {
    Cont(T),
    Term,
}

#[derive(Debug, Clone)]
pub struct MatchmakingState<Taker: Stable, Maker: Stable> {
    pub terminal_makes: HashMap<Taker::StableId, (Make, Next<Taker>)>,
    pub takes: HashMap<Maker::StableId, (Take, Next<Maker>)>,
    pub remainder: Option<MakeInProgress<Taker>>,
}

impl<Taker: Stable, Maker: Stable> MatchmakingState<Taker, Maker> {
    pub fn new(initial_taker: Taker) -> Self {
        Self {
            terminal_makes: HashMap::new(),
            takes: HashMap::new(),
            remainder: Some(MakeInProgress::new(initial_taker)),
        }
    }
}
