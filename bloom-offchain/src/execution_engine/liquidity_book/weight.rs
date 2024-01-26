use std::cmp::Ordering;

use num_rational::Ratio;

use crate::execution_engine::liquidity_book::fragment::Fragment;
use crate::execution_engine::liquidity_book::types::{ExCostUnits, FeeAsset};

#[derive(Copy, Clone, Eq, PartialEq)]
pub struct OrderWeight(Ratio<u64>, ExCostUnits);

impl PartialOrd for OrderWeight {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        match PartialOrd::partial_cmp(&self.0, &other.0) {
            Some(Ordering::Equal) => PartialOrd::partial_cmp(&self.1, &other.1).map(|x| x.reverse()),
            cmp => cmp,
        }
    }
}

impl Ord for OrderWeight {
    fn cmp(&self, other: &Self) -> Ordering {
        match Ord::cmp(&self.0, &other.0) {
            Ordering::Equal => Ord::cmp(&self.1, &other.1).reverse(),
            cmp => cmp,
        }
    }
}

impl OrderWeight {
    pub fn new(fee: FeeAsset<Ratio<u64>>, cost: ExCostUnits) -> Self {
        Self(fee, cost)
    }
}

pub trait Weighted {
    fn weight(&self) -> OrderWeight;
}

impl<T> Weighted for T
where
    T: Fragment,
{
    fn weight(&self) -> OrderWeight {
        OrderWeight(self.weighted_fee(), self.marginal_cost_hint())
    }
}

#[cfg(test)]
mod tests {
    use num_rational::Ratio;

    use crate::execution_engine::liquidity_book::weight::OrderWeight;

    #[test]
    fn order_with_lower_cost_is_preferred() {
        let w1 = OrderWeight::new(Ratio::new(99, 1000), 1000);
        let w2 = OrderWeight::new(Ratio::new(99, 1000), 1001);
        assert!(w1 > w2);
    }
}
