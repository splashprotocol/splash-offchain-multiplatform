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
    side: Side,
}

#[derive(Clone, Debug, Serialize)]
pub struct ExecutionReportPartial<I, V, Pair, Meta> {
    pair: Pair,
    executions: Vec<OrderExecution<I, V>>,
    meta: Meta,
}

#[derive(Clone, Debug, Serialize)]
pub struct ExecutionReport<I, V, TxHash, Pair, Meta> {
    pair: Pair,
    executions: Vec<OrderExecution<I, V>>,
    meta: Meta,
    tx_hash: TxHash,
}

impl<I, V, Pair, Meta> ExecutionReportPartial<I, V, Pair, Meta> {
    pub fn new<T: MarketTaker + Stable<StableId = I>, M, B: Has<V>>(
        ExecutionRecipe(instructions): &ExecutionRecipe<T, M, B>,
        pair: Pair,
        meta: Meta,
    ) -> Self {
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
                        side: target.side(),
                    });
                }
                Either::Right(_) => {}
            }
        }
        Self {
            pair,
            executions,
            meta,
        }
    }

    pub fn finalize<TxHash>(self, tx_hash: TxHash) -> ExecutionReport<I, V, TxHash, Pair, Meta> {
        ExecutionReport {
            pair: self.pair,
            executions: self.executions,
            meta: self.meta,
            tx_hash,
        }
    }
}
