use bloom_offchain::execution_engine::liquidity_book::core::ExecutionMeta;
use bloom_offchain::execution_engine::liquidity_book::side::Side;
use bloom_offchain::execution_engine::liquidity_book::types::AbsolutePrice;
use cml_crypto::TransactionHash;
use serde::{Deserialize, Serialize};
use spectrum_cardano_lib::{OutputRef, Token};
use spectrum_offchain_cardano::data::pair::PairId;

#[derive(Copy, Clone, Eq, PartialEq, Debug, Serialize, Deserialize)]
pub struct OrderExecution {
    pub id: Token,
    pub version: OutputRef,
    pub mean_price: AbsolutePrice,
    pub removed_input: u64,
    pub added_output: u64,
    pub side: Side,
}

#[derive(Clone, Eq, PartialEq, Debug, Serialize, Deserialize)]
pub struct ExecutionReport {
    pub pair: PairId,
    pub executions: Vec<OrderExecution>,
    pub meta: ExecutionMeta,
    pub tx_hash: TransactionHash,
}

#[cfg(test)]
mod tests {
    use crate::message::{ExecutionReport, OrderExecution};
    use bloom_offchain::execution_engine::liquidity_book::core::ExecutionMeta;
    use bloom_offchain::execution_engine::liquidity_book::side::Side;
    use bloom_offchain::execution_engine::liquidity_book::types::AbsolutePrice;
    use cml_crypto::TransactionHash;
    use spectrum_cardano_lib::{AssetClass, OutputRef, Token};
    use spectrum_offchain_cardano::data::pair::PairId;

    #[test]
    fn sample_report_json_roundtrip() {
        let pair = PairId::canonical(
            AssetClass::Native,
            AssetClass::Token(Token::from_string_unsafe(
                "d3917756db639cf4a7dbc2a61684fe1fc99862e0da81708c2493c41f.434154",
            )),
        );
        let order_execution = OrderExecution {
            id: Token::from_string_unsafe("d3917756db639cf4a7dbc2a61684fe1fc99862e0da81708c2493c41f.434154"),
            version: OutputRef::from_string_unsafe(
                "59811364865a45bc001dd81f8c7bf21bf74749f84d7a8a061505a38c14fec544#0",
            ),
            mean_price: AbsolutePrice::new_unsafe(1, 2),
            removed_input: 1,
            added_output: 2,
            side: Side::Bid,
        };
        let report = ExecutionReport {
            pair,
            executions: vec![order_execution],
            meta: ExecutionMeta {
                mean_spot_price: Some(AbsolutePrice::new_unsafe(1, 2).into()),
            },
            tx_hash: TransactionHash::from_hex(
                "8064bf12c840f8c5abd319359a31d181c5bec3237b903fa8577a081669638a08",
            )
            .unwrap(),
        };
        let report_json = serde_json::to_string(&report).unwrap();
        println!("{}", report_json);
        let decoded_report = serde_json::from_str::<ExecutionReport>(&report_json).unwrap();
        println!("{:?}", decoded_report);
        assert_eq!(report, decoded_report);
    }
}
