use std::cmp::Ordering;

use crate::execution_engine::liquidity_book::market_taker::MarketTaker;
use crate::execution_engine::liquidity_book::types::FeeAsset;

#[derive(Copy, Clone, Eq, PartialEq)]
pub struct OrderWeight<CostUnits>(u64, CostUnits);

impl<U: PartialOrd> PartialOrd for OrderWeight<U> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        match PartialOrd::partial_cmp(&self.0, &other.0) {
            Some(Ordering::Equal) => PartialOrd::partial_cmp(&self.1, &other.1).map(|x| x.reverse()),
            cmp => cmp,
        }
    }
}

impl<U: Ord> Ord for OrderWeight<U> {
    fn cmp(&self, other: &Self) -> Ordering {
        match Ord::cmp(&self.0, &other.0) {
            Ordering::Equal => Ord::cmp(&self.1, &other.1).reverse(),
            cmp => cmp,
        }
    }
}

impl<U> OrderWeight<U> {
    pub fn new(fee: FeeAsset<u64>, cost: U) -> Self {
        Self(fee, cost)
    }
}

pub trait Weighted<U> {
    fn weight(&self) -> OrderWeight<U>;
}

impl<T, U> Weighted<U> for T
where
    T: MarketTaker<U = U>,
{
    fn weight(&self) -> OrderWeight<U> {
        OrderWeight(self.fee(), self.marginal_cost_hint())
    }
}

#[cfg(test)]
mod tests {
    use crate::execution_engine::liquidity_book::weight::OrderWeight;

    #[test]
    fn order_with_lower_cost_is_preferred() {
        let w1 = OrderWeight::new(100, 1000);
        let w2 = OrderWeight::new(100, 1001);
        assert!(w1 > w2);
    }
}
