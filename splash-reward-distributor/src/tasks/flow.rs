use crate::onchain::buffer_wallet::WalletId;

pub enum Flow<FarmId, OrderId> {
    GaugeBuffering(GaugeBuffering<FarmId>),
    Harvesting(Harvesting<OrderId>),
}

pub struct GaugeBuffering<FarmId> {
    pub gauge: FarmId,
    pub wallet: WalletId,
}

pub struct Harvesting<Id> {
    pub order: Id,
}
