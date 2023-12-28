use std::collections::HashMap;

use spectrum_offchain::data::order::OrderLink;
use spectrum_offchain::data::SpecializedOrder;

pub trait HotOrderRegistry<TOrd: SpecializedOrder> {
    fn register(&mut self, order_link: OrderLink<TOrd>);
    fn deregister(&mut self, order_id: TOrd::TOrderId) -> Option<OrderLink<TOrd>>;
}

#[derive(Clone)]
pub struct EphemeralHotOrderRegistry<TOrd: SpecializedOrder> {
    store: HashMap<TOrd::TOrderId, TOrd::TPoolId>,
}

impl<TOrd: SpecializedOrder> EphemeralHotOrderRegistry<TOrd> {
    pub fn new() -> Self {
        Self {
            store: HashMap::new(),
        }
    }
}

impl<TOrd> HotOrderRegistry<TOrd> for EphemeralHotOrderRegistry<TOrd>
where
    TOrd: SpecializedOrder,
    TOrd::TPoolId: Copy,
{
    fn register(&mut self, OrderLink { order_id, pool_id }: OrderLink<TOrd>) {
        self.store.insert(order_id, pool_id);
    }

    fn deregister(&mut self, order_id: TOrd::TOrderId) -> Option<OrderLink<TOrd>> {
        self.store
            .remove(&order_id)
            .map(|v| OrderLink { order_id, pool_id: v })
    }
}
