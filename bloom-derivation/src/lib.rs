extern crate quote;
extern crate syn;

use proc_macro::TokenStream;

use derive_utils::quick_derive;

#[proc_macro_derive(MarketTaker)]
pub fn derive_market_taker(input: TokenStream) -> TokenStream {
    quick_derive! {
        input,
        bloom_offchain::execution_engine::liquidity_book::market_taker::MarketTaker,
        pub trait MarketTaker {
            type U;
            fn side(&self) -> bloom_offchain::execution_engine::liquidity_book::side::Side;
            fn input(&self) -> bloom_offchain::execution_engine::liquidity_book::types::InputAsset<u64>;
            fn output(&self) -> bloom_offchain::execution_engine::liquidity_book::types::OutputAsset<u64>;
            fn price(&self) -> bloom_offchain::execution_engine::liquidity_book::types::AbsolutePrice;
            fn operator_fee(&self, input_consumed: bloom_offchain::execution_engine::liquidity_book::types::InputAsset<u64>) -> bloom_offchain::execution_engine::liquidity_book::types::FeeAsset<u64>;
            fn fee(&self) -> bloom_offchain::execution_engine::liquidity_book::types::FeeAsset<u64>;
            fn budget(&self) -> bloom_offchain::execution_engine::liquidity_book::types::FeeAsset<u64>;
            fn consumable_budget(&self) -> bloom_offchain::execution_engine::liquidity_book::types::FeeAsset<u64>;
            fn marginal_cost_hint(&self) -> Self::U;
            fn time_bounds(&self) -> bloom_offchain::execution_engine::liquidity_book::time::TimeBounds<u64>;
            fn min_marginal_output(&self) -> bloom_offchain::execution_engine::liquidity_book::types::OutputAsset<u64>;
        }
    }
}

#[proc_macro_derive(Stable)]
pub fn derive_stable(input: TokenStream) -> TokenStream {
    quick_derive! {
        input,
        spectrum_offchain::domain::Stable,
        pub trait Stable {
            type StableId: Copy + Eq + Hash + Display;
            fn stable_id(&self) -> Self::StableId;
            fn is_quasi_permanent(&self) -> bool;
        }
    }
}

#[proc_macro_derive(EntitySnapshot)]
pub fn derive_entity_snapshot(input: TokenStream) -> TokenStream {
    quick_derive! {
        input,
        spectrum_offchain::domain::EntitySnapshot,
        pub trait EntitySnapshot {
            type Version: Copy + Eq + Hash + Display;
            fn version(&self) -> Self::Version;
        }
    }
}

#[proc_macro_derive(Tradable)]
pub fn derive_tradable(input: TokenStream) -> TokenStream {
    quick_derive! {
        input,
        spectrum_offchain::domain::Tradable,
        pub trait Tradable {
            type PairId: Copy + Eq + Hash + Display;
            fn pair_id(&self) -> Self::PairId;
        }
    }
}
