use bloom_offchain::execution_engine::liquidity_book::core::ExecutionMeta;
use bloom_offchain::execution_engine::liquidity_book::side::Side;
use bloom_offchain::execution_engine::liquidity_book::types::AbsolutePrice;
use cml_crypto::TransactionHash;
use serde::{Deserialize, Serialize};
use spectrum_cardano_lib::{OutputRef, Token};
use spectrum_offchain_cardano::data::pair::PairId;

#[derive(Copy, Clone, Debug, Serialize, Deserialize)]
pub struct OrderExecution {
    id: Token,
    version: OutputRef,
    mean_price: AbsolutePrice,
    removed_input: u64,
    added_output: u64,
    side: Side,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ExecutionReport {
    pair: PairId,
    executions: Vec<OrderExecution>,
    meta: ExecutionMeta,
    tx_hash: TransactionHash,
}
