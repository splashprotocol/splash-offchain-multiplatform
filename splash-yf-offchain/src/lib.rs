pub mod entities;
pub mod events;
pub mod settings;

#[derive(
    Copy,
    Clone,
    Debug,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    serde::Serialize,
    serde::Deserialize,
    derive_more::From,
    derive_more::Into,
    derive_more::Display,
)]
pub struct Epoch(u64);
