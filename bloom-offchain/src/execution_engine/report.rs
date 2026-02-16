use crate::execution_engine::bundled::Bundled;
use crate::execution_engine::liquidity_book::core::ExecutionRecipe;
use crate::execution_engine::liquidity_book::market_taker::MarketTaker;
use crate::execution_engine::liquidity_book::side::Side;
use crate::execution_engine::liquidity_book::types::AbsolutePrice;
use either::Either;
use num_rational::Ratio;
use serde::Serialize;
use spectrum_offchain::domain::{Has, Stable};

#[derive(Copy, Clone, Debug, Serialize)]
pub struct OrderExecution<I, V> {
    id: I,
    version: V,
    mean_price: AbsolutePrice,
    removed_input: u64,
    added_output: u64,
    fee: u64,
    side: Side,
}

/// Report of an execution attempt on the liquidity book.
#[derive(Clone, Debug, Serialize)]
pub struct ExecutionReport<I, V, TxHash, Pair, Events> {
    pair: Pair,
    executions: Vec<OrderExecution<I, V>>,
    events: Events,
    tx_hash: Option<TxHash>,
}

impl<I, V, TxHash, Pair, Events> ExecutionReport<I, V, TxHash, Pair, Events> {
    pub fn new(pair: Pair, events: Events) -> Self {
        Self {
            pair,
            executions: Vec::new(),
            events,
            tx_hash: None,
        }
    }

    pub fn with_executions<T: MarketTaker + Stable<StableId = I>, M, B: Has<V>>(
        &mut self,
        ExecutionRecipe(instructions): &ExecutionRecipe<T, M, B>,
    ) {
        let mut executions = Vec::with_capacity(instructions.len());
        for instruction in instructions {
            match instruction {
                Either::Left(take) => {
                    let Bundled(target, br) = &take.target;
                    let input = take.removed_input();
                    let output = take.added_output();
                    let side = target.side();
                    let rel_price = Ratio::new(output as u128, input as u128);
                    executions.push(OrderExecution {
                        id: target.stable_id(),
                        version: br.get(),
                        mean_price: AbsolutePrice::from_price(side, rel_price),
                        added_output: input,
                        removed_input: output,
                        fee: take.consumed_fee(),
                        side: target.side(),
                    });
                }
                Either::Right(_) => {}
            }
        }
        self.executions = executions;
    }

    pub fn finalized(&mut self, tx_hash: TxHash) {
        self.tx_hash = Some(tx_hash);
    }
}
