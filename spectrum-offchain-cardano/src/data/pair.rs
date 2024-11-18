use std::fmt::{Display, Formatter, Write};

use num_rational::Ratio;

use bloom_offchain::execution_engine::liquidity_book::side::Side;
use spectrum_cardano_lib::AssetClass;

#[derive(Debug, Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct PairId(AssetClass, AssetClass);

impl PairId {
    /// Build canonical pair.
    pub fn canonical(x: AssetClass, y: AssetClass) -> Self {
        let xs = order_canonical(x, y);
        Self(xs[0], xs[1])
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
        f.write_str(format!("[{}]/[{}]", self.0, self.1).as_str())
    }
}
/// Returns relative price, i.e. relative to the given trade Side and (x, y) pair canonical ordering.

pub fn absolute_price_to_relative(
    absolute_ratio: Ratio<u128>,
    x: AssetClass,
    y: AssetClass,
    side: Side,
) -> (u128, u128) {
    let [base, _] = order_canonical(x, y);
    let x_is_base = x == base;
    let (ratio_canonical_num, ratio_canonical_denom) = (absolute_ratio.numer(), absolute_ratio.denom());
    let (ratio_relative_num, ratio_relative_denom) = match side {
        Side::Ask => {
            if x_is_base {
                (ratio_canonical_num, ratio_canonical_denom)
            } else {
                (ratio_canonical_denom, ratio_canonical_num)
            }
        }
        Side::Bid => {
            if x_is_base {
                (ratio_canonical_denom, ratio_canonical_num)
            } else {
                (ratio_canonical_num, ratio_canonical_denom)
            }
        }
    };
    (*ratio_relative_num, *ratio_relative_denom)
}
