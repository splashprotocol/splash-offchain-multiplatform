use crate::onchain::buffer_wallet::BufferWallet;
use crate::onchain::harvest_order::HarvestOrder;
use crate::onchain::smart_farm::Gauge;
use bloom_offchain::execution_engine::bundled::Bundled;

#[derive(Debug, Clone, PartialEq)]
pub struct HarvestBatch<StateId, Bearer> {
    pub buffer_wallet: Bundled<BufferWallet<StateId>, Bearer>,
    pub orders: Vec<OrderWithPayout<StateId, Bearer>>,
    pub total_payout: u64,
}

impl<StateId, Bearer> HarvestBatch<StateId, Bearer> {
    pub fn new(buffer_wallet: Bundled<BufferWallet<StateId>, Bearer>) -> Self {
        Self {
            buffer_wallet,
            orders: vec![],
            total_payout: 0,
        }
    }

    pub fn can_accept(&self, payout: u64) -> bool {
        self.total_payout + payout <= self.buffer_wallet.0.balance
    }

    pub fn add_order(&mut self, order: Bundled<HarvestOrder<StateId>, Bearer>, payout: u64) {
        let order = OrderWithPayout { order, payout };
        self.orders.push(order);
        self.total_payout += payout;
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct OrderWithPayout<StateId, Bearer> {
    pub order: Bundled<HarvestOrder<StateId>, Bearer>,
    pub payout: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BufferingBatch<GaugeId, StateId, Bearer> {
    pub gauges: Vec<Bundled<Gauge<GaugeId, StateId>, Bearer>>,
    pub buffer_wallet: Bundled<BufferWallet<StateId>, Bearer>,
}

impl<GaugeId, StateId, Bearer> BufferingBatch<GaugeId, StateId, Bearer> {
    pub fn new(buffer_wallet: Bundled<BufferWallet<StateId>, Bearer>) -> Self {
        Self {
            buffer_wallet,
            gauges: vec![],
        }
    }

    pub fn gauges(&self) -> Vec<&Gauge<GaugeId, StateId>> {
        self.gauges.iter().map(|Bundled(t, _)| t).collect()
    }

    pub fn add_gauge(&mut self, gauge: Bundled<Gauge<GaugeId, StateId>, Bearer>) {
        self.gauges.push(gauge);
    }
}
