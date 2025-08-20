#[derive(
    Debug,
    Clone,
    Copy,
    serde::Deserialize,
    serde::Serialize,
    Eq,
    PartialEq,
    derive_more::From,
    derive_more::Into,
)]
pub struct MinLovelacePerHarvest(pub u64);
