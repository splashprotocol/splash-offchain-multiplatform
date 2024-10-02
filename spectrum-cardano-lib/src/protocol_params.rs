use cml_chain::builders::tx_builder::{TransactionBuilder, TransactionBuilderConfigBuilder};
use cml_chain::fees::LinearFee;
use cml_chain::plutus::{CostModels, ExUnitPrices};
use cml_chain::SubCoin;
use cml_core::ordered_hash_map::OrderedHashMap;

const MAX_TX_SIZE: u32 = 16384;
const MAX_VALUE_SIZE: u32 = 5000;

pub const COINS_PER_UTXO_BYTE: u64 = 4310;

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
        100788, 420, 1, 1, 1000, 173, 0, 1, 1000, 59957, 4, 1, 11183, 32, 201305, 8356, 4, 16000, 100, 16000,
        100, 16000, 100, 16000, 100, 16000, 100, 16000, 100, 100, 100, 16000, 100, 94375, 32, 132994, 32,
        61462, 4, 72010, 178, 0, 1, 22151, 32, 91189, 769, 4, 2, 85848, 228465, 122, 0, 1, 1, 1000, 42921, 4,
        2, 24548, 29498, 38, 1, 898148, 27279, 1, 51775, 558, 1, 39184, 1000, 60594, 1, 141895, 32, 83150,
        32, 15299, 32, 76049, 1, 13169, 4, 22100, 10, 28999, 74, 1, 28999, 74, 1, 43285, 552, 1, 44749, 541,
        1, 33852, 32, 68246, 32, 72362, 32, 7243, 32, 7391, 32, 11546, 32, 85848, 228465, 122, 0, 1, 1,
        90434, 519, 0, 1, 74433, 32, 85848, 228465, 122, 0, 1, 1, 85848, 228465, 122, 0, 1, 1, 270652, 22588,
        4, 1457325, 64566, 4, 20467, 1, 4, 0, 141992, 32, 100788, 420, 1, 1, 81663, 32, 59498, 32, 20142, 32,
        24588, 32, 20744, 32, 25933, 32, 24623, 32, 53384111, 14333, 10,
    ];

    let ops_v2 = [
        100788, 420, 1, 1, 1000, 173, 0, 1, 1000, 59957, 4, 1, 11183, 32, 201305, 8356, 4, 16000, 100, 16000,
        100, 16000, 100, 16000, 100, 16000, 100, 16000, 100, 100, 100, 16000, 100, 94375, 32, 132994, 32,
        61462, 4, 72010, 178, 0, 1, 22151, 32, 91189, 769, 4, 2, 85848, 228465, 122, 0, 1, 1, 1000, 42921, 4,
        2, 24548, 29498, 38, 1, 898148, 27279, 1, 51775, 558, 1, 39184, 1000, 60594, 1, 141895, 32, 83150,
        32, 15299, 32, 76049, 1, 13169, 4, 22100, 10, 28999, 74, 1, 28999, 74, 1, 43285, 552, 1, 44749, 541,
        1, 33852, 32, 68246, 32, 72362, 32, 7243, 32, 7391, 32, 11546, 32, 85848, 228465, 122, 0, 1, 1,
        90434, 519, 0, 1, 74433, 32, 85848, 228465, 122, 0, 1, 1, 85848, 228465, 122, 0, 1, 1, 955506,
        213312, 0, 2, 270652, 22588, 4, 1457325, 64566, 4, 20467, 1, 4, 0, 141992, 32, 100788, 420, 1, 1,
        81663, 32, 59498, 32, 20142, 32, 24588, 32, 20744, 32, 25933, 32, 24623, 32, 43053543, 10, 53384111,
        14333, 10, 43574283, 26308, 10,
    ];

    let ops_v3 = [
        100788, 420, 1, 1, 1000, 173, 0, 1, 1000, 59957, 4, 1, 11183, 32, 201305, 8356, 4, 16000, 100, 16000,
        100, 16000, 100, 16000, 100, 16000, 100, 16000, 100, 100, 100, 16000, 100, 94375, 32, 132994, 32,
        61462, 4, 72010, 178, 0, 1, 22151, 32, 91189, 769, 4, 2, 85848, 123203, 7305, -900, 1716, 549, 57,
        85848, 0, 1, 1, 1000, 42921, 4, 2, 24548, 29498, 38, 1, 898148, 27279, 1, 51775, 558, 1, 39184, 1000,
        60594, 1, 141895, 32, 83150, 32, 15299, 32, 76049, 1, 13169, 4, 22100, 10, 28999, 74, 1, 28999, 74,
        1, 43285, 552, 1, 44749, 541, 1, 33852, 32, 68246, 32, 72362, 32, 7243, 32, 7391, 32, 11546, 32,
        85848, 123203, 7305, -900, 1716, 549, 57, 85848, 0, 1, 90434, 519, 0, 1, 74433, 32, 85848, 123203,
        7305, -900, 1716, 549, 57, 85848, 0, 1, 1, 85848, 123203, 7305, -900, 1716, 549, 57, 85848, 0, 1,
        955506, 213312, 0, 2, 270652, 22588, 4, 1457325, 64566, 4, 20467, 1, 4, 0, 141992, 32, 100788, 420,
        1, 1, 81663, 32, 59498, 32, 20142, 32, 24588, 32, 20744, 32, 25933, 32, 24623, 32, 43053543, 10,
        53384111, 14333, 10, 43574283, 26308, 10, 16000, 100, 16000, 100, 962335, 18, 2780678, 6, 442008, 1,
        52538055, 3756, 18, 267929, 18, 76433006, 8868, 18, 52948122, 18, 1995836, 36, 3227919, 12, 901022,
        1, 166917843, 4307, 36, 284546, 36, 158221314, 26549, 36, 74698472, 36, 333849714, 1, 254006273, 72,
        2174038, 72, 2261318, 64571, 4, 207616, 8310, 4, 1293828, 28716, 63, 0, 1, 1006041, 43623, 251, 0, 1,
    ];

    let mut cost_models_map: OrderedHashMap<u64, Vec<i64>> = OrderedHashMap::new();
    cost_models_map.insert(0, ops_v1.into());
    cost_models_map.insert(1, ops_v2.into());
    cost_models_map.insert(2, ops_v3.into());

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
