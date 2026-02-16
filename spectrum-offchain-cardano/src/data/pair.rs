use bloom_offchain::execution_engine::liquidity_book::side::Side;
use serde::{Deserialize, Serialize};
use spectrum_cardano_lib::AssetClass;
use std::fmt::{Display, Formatter};

#[derive(Debug, Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Serialize, Deserialize)]
pub struct PairId(AssetClass, AssetClass);

impl PairId {
    /// Build canonical pair.
    pub fn canonical(x: AssetClass, y: AssetClass) -> Self {
        let xs = order_canonical(x, y);
        Self(xs[0], xs[1])
    }

    pub fn dummy() -> Self {
        Self(AssetClass::Native, AssetClass::Native)
    }

    /// Access the two AssetClass components of this pair (Base, Quote) in canonical order.
    pub fn assets(&self) -> (AssetClass, AssetClass) {
        (self.0, self.1)
    }
}

/// Determine side of a trade relatively to canonical pair.
pub fn side_of(input: AssetClass, output: AssetClass) -> Side {
    let xs = order_canonical(input, output);
    if xs[0] == input {
        Side::Ask
    } else {
        Side::Bid
    }
}

/// Returns two given [AssetClass] ordered as 2-array where the first element
/// is Base asset in canonical pair, and the second is Quote.
pub fn order_canonical(x: AssetClass, y: AssetClass) -> [AssetClass; 2] {
    let mut bf = [x, y];
    bf.sort();
    bf
}

impl Display for PairId {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(format!("({}, {})", self.0, self.1).as_str())
    }
}
