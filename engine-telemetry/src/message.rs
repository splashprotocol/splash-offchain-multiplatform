use bloom_offchain::execution_engine::liquidity_book::core::ExecutionMeta;
use bloom_offchain::execution_engine::liquidity_book::side::Side;
use bloom_offchain::execution_engine::liquidity_book::types::AbsolutePrice;
use cml_crypto::TransactionHash;
use serde::{Deserialize, Serialize};
use spectrum_cardano_lib::{OutputRef, Token};
use spectrum_offchain_cardano::data::pair::PairId;

#[derive(Copy, Clone, Debug, Serialize, Deserialize)]
pub struct OrderExecution {
    pub id: Token,
    pub version: OutputRef,
    pub mean_price: AbsolutePrice,
    pub removed_input: u64,
    pub added_output: u64,
    pub side: Side,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ExecutionReport {
    pub pair: PairId,
    pub executions: Vec<OrderExecution>,
    pub meta: ExecutionMeta,
    pub tx_hash: TransactionHash,
}
