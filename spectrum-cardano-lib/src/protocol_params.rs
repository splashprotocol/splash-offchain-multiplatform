use cml_chain::builders::tx_builder::{TransactionBuilder, TransactionBuilderConfigBuilder};
use cml_chain::fees::LinearFee;
use cml_chain::plutus::{CostModels, ExUnitPrices};
use cml_chain::SubCoin;
use cml_core::ordered_hash_map::OrderedHashMap;

const MAX_TX_SIZE: u32 = 16384;
const MAX_VALUE_SIZE: u32 = 5000;

const COINS_PER_UTXO_BYTE: u64 = 4310;

pub fn constant_tx_builder() -> TransactionBuilder {
    create_tx_builder_full(
        LinearFee::new(44, 155381, 15),
        500000000,
        2000000,
        MAX_VALUE_SIZE,
        COINS_PER_UTXO_BYTE,
    )
}

pub fn constant_cost_models() -> CostModels {
    let ops_v1 = [
        205665, 812, 1, 1, 1000, 571, 0, 1, 1000, 24177, 4, 1, 1000, 32, 117366, 10475, 4, 23000, 100, 23000,
        100, 23000, 100, 23000, 100, 23000, 100, 23000, 100, 100, 100, 23000, 100, 19537, 32, 175354, 32,
        46417, 4, 221973, 511, 0, 1, 89141, 32, 497525, 14068, 4, 2, 196500, 453240, 220, 0, 1, 1, 1000,
        28662, 4, 2, 245000, 216773, 62, 1, 1060367, 12586, 1, 208512, 421, 1, 187000, 1000, 52998, 1, 80436,
        32, 43249, 32, 1000, 32, 80556, 1, 57667, 4, 1000, 10, 197145, 156, 1, 197145, 156, 1, 204924, 473,
        1, 208896, 511, 1, 52467, 32, 64832, 32, 65493, 32, 22558, 32, 16563, 32, 76511, 32, 196500, 453240,
        220, 0, 1, 1, 69522, 11687, 0, 1, 60091, 32, 196500, 453240, 220, 0, 1, 1, 196500, 453240, 220, 0, 1,
        1, 806990, 30482, 4, 1927926, 82523, 4, 265318, 0, 4, 0, 85931, 32, 205665, 812, 1, 1, 41182, 32,
        212342, 32, 31220, 32, 32696, 32, 43357, 32, 32247, 32, 38314, 32, 57996947, 18975, 10,
    ];

    let ops_v2 = [
        205665, 812, 1, 1, 1000, 571, 0, 1, 1000, 24177, 4, 1, 1000, 32, 117366, 10475, 4, 23000, 100, 23000,
        100, 23000, 100, 23000, 100, 23000, 100, 23000, 100, 100, 100, 23000, 100, 19537, 32, 175354, 32,
        46417, 4, 221973, 511, 0, 1, 89141, 32, 497525, 14068, 4, 2, 196500, 453240, 220, 0, 1, 1, 1000,
        28662, 4, 2, 245000, 216773, 62, 1, 1060367, 12586, 1, 208512, 421, 1, 187000, 1000, 52998, 1, 80436,
        32, 43249, 32, 1000, 32, 80556, 1, 57667, 4, 1000, 10, 197145, 156, 1, 197145, 156, 1, 204924, 473,
        1, 208896, 511, 1, 52467, 32, 64832, 32, 65493, 32, 22558, 32, 16563, 32, 76511, 32, 196500, 453240,
        220, 0, 1, 1, 69522, 11687, 0, 1, 60091, 32, 196500, 453240, 220, 0, 1, 1, 196500, 453240, 220, 0, 1,
        1, 1159724, 392670, 0, 2, 806990, 30482, 4, 1927926, 82523, 4, 265318, 0, 4, 0, 85931, 32, 205665,
        812, 1, 1, 41182, 32, 212342, 32, 31220, 32, 32696, 32, 43357, 32, 32247, 32, 38314, 32, 35892428,
        10, 57996947, 18975, 10, 38887044, 32947, 10,
    ];

    let mut cost_models_map: OrderedHashMap<u64, Vec<i64>> = OrderedHashMap::new();
    cost_models_map.insert(0, ops_v1.into());
    cost_models_map.insert(1, ops_v2.into());

    CostModels::new(cost_models_map)
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
        .cost_models(constant_cost_models())
        .build()
        .unwrap();
    TransactionBuilder::new(cfg)
}
