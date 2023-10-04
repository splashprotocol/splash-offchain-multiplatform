use cml_chain::builders::tx_builder::{TransactionBuilder, TransactionBuilderConfigBuilder};
use cml_chain::fees::LinearFee;
use cml_chain::genesis::network_info::plutus_alonzo_cost_models;
use cml_chain::plutus::ExUnitPrices;
use cml_chain::SubCoin;

const MAX_TX_SIZE: u32 = 8000;
const MAX_VALUE_SIZE: u32 = 4000;

const COINS_PER_UTXO_BYTE: u64 = 4310;

//todo: check correctness
pub fn constant_tx_builder() -> TransactionBuilder {
    create_tx_builder_full(
        LinearFee::new(44, 155381),
        500000000,
        2000000,
        MAX_VALUE_SIZE,
        COINS_PER_UTXO_BYTE,
    )
}

fn create_tx_builder_full(
    linear_fee: LinearFee,
    pool_deposit: u64,
    key_deposit: u64,
    max_val_size: u32,
    coins_per_utxo_byte: u64,
) -> TransactionBuilder {
    let cfg = TransactionBuilderConfigBuilder::default()
        .fee_algo(linear_fee)
        .pool_deposit(pool_deposit)
        .key_deposit(key_deposit)
        .max_value_size(max_val_size)
        .max_tx_size(MAX_TX_SIZE)
        .coins_per_utxo_byte(coins_per_utxo_byte)
        .ex_unit_prices(ExUnitPrices::new(
            SubCoin::new(577, 10000),
            SubCoin::new(721, 10000000),
        ))
        .collateral_percentage(150)
        .max_collateral_inputs(3)
        .cost_models(plutus_alonzo_cost_models())
        .build()
        .unwrap();
    TransactionBuilder::new(cfg)
}