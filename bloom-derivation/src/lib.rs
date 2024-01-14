extern crate quote;
extern crate syn;

use proc_macro::TokenStream;

use derive_utils::quick_derive;

#[proc_macro_derive(Fragment)]
pub fn derive_fragment(input: TokenStream) -> TokenStream {
    quick_derive! {
        input,
        bloom_offchain::execution_engine::liquidity_book::fragment::Fragment,
        pub trait Fragment {
            fn side(&self) -> bloom_offchain::execution_engine::liquidity_book::side::SideM;
            fn input(&self) -> u64;
            fn price(&self) -> bloom_offchain::execution_engine::liquidity_book::types::AbsolutePrice;
            fn fee(&self) -> FeePerOutput;
            fn cost_hint(&self) -> bloom_offchain::execution_engine::liquidity_book::types::ExecutionCost;
            fn time_bounds(&self) -> bloom_offchain::execution_engine::liquidity_book::time::TimeBounds<u64>;
        }
    }
}

#[proc_macro_derive(Stable)]
pub fn derive_stable(input: TokenStream) -> TokenStream {
    quick_derive! {
        input,
        spectrum_offchain::data::Stable,
        pub trait Stable {
            type StableId: Copy + Eq + Hash + Display;
            fn stable_id(&self) -> Self::StableId;
        }
    }
}

#[proc_macro_derive(EntitySnapshot)]
pub fn derive_entity_snapshot(input: TokenStream) -> TokenStream {
    quick_derive! {
        input,
        spectrum_offchain::data::EntitySnapshot,
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
        spectrum_offchain::data::Tradable,
        pub trait Tradable {
            type PairId: Copy + Eq + Hash + Display;
            fn pair_id(&self) -> Self::PairId;
        }
    }
}
