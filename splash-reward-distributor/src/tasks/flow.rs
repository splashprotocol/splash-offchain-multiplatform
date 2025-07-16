pub enum Flow<FarmId, OrderId> {
    GaugeBuffering(GaugeBuffering<FarmId>),
    Harvesting(Harvesting<OrderId>),
}

pub struct GaugeBuffering<FarmId> {
    pub gauge: FarmId,
}

pub struct Harvesting<Id> {
    pub order: Id,
}
