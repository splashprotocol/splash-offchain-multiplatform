use std::fmt::Display;

#[derive(
    Copy, Clone, Debug, PartialEq, Eq, Hash, Default, serde::Serialize, serde::Deserialize, derive_more::From,
)]
pub struct TaskId([u8; 32]);

impl Display for TaskId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", hex::encode(self.0))
    }
}

#[derive(Copy, Clone)]
pub enum Task<GaugeId, OrderId> {
    GaugeBuffering(GaugeBuffering<GaugeId>),
    Harvesting(Harvesting<OrderId>),
}

impl<GaugeId, OrderId> Task<GaugeId, OrderId> {
    pub fn new_gauge_buffering(gauge: GaugeId) -> Self {
        Self::GaugeBuffering(GaugeBuffering { gauge_id: gauge })
    }

    pub fn new_harvesting(order: OrderId) -> Self {
        Self::Harvesting(Harvesting { order_id: order })
    }
}

#[derive(Copy, Clone)]
pub struct GaugeBuffering<GaugeId> {
    pub gauge_id: GaugeId,
}

#[derive(Copy, Clone)]
pub struct Harvesting<OrderId> {
    pub order_id: OrderId,
}
