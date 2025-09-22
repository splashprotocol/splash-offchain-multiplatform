use bloom_offchain::execution_engine::bundled::Bundled;
use rs_merkle::algorithms::Keccak256;
use rs_merkle::MerkleTree;
use splash_yf_offchain::entities::auth_manager::AuthManager;
use splash_yf_offchain::entities::buffer_wallet::BufferWallet;
use splash_yf_offchain::entities::gauge::Gauge;
use splash_yf_offchain::entities::harvest_order::HarvestOrder;

use crate::entity_index::HarvestOrderSpend;

#[derive(Clone)]
pub struct HarvestBatch<StateId, Bearer> {
    pub buffer_wallet: Bundled<BufferWallet<StateId>, Bearer>,
    pub orders: Vec<OrderWithSpendDetails<StateId, Bearer>>,
    pub input_merkle_tree: MerkleTree<Keccak256>,
    pub total_payout: u64,
}

impl<StateId, Bearer> HarvestBatch<StateId, Bearer> {
    pub fn new(
        buffer_wallet: Bundled<BufferWallet<StateId>, Bearer>,
        input_merkle_tree: MerkleTree<Keccak256>,
    ) -> Self {
        Self {
            buffer_wallet,
            input_merkle_tree,
            orders: vec![],
            total_payout: 0,
        }
    }

    pub fn can_accept(&self, payout: u64) -> bool {
        self.total_payout + payout <= self.buffer_wallet.0.balance
    }

    pub fn add_order(&mut self, order: OrderWithSpendDetails<StateId, Bearer>) {
        self.total_payout += order.spend.amount;
        self.orders.push(order);
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct OrderWithSpendDetails<StateId, Bearer> {
    pub order_bundle: Bundled<HarvestOrder<StateId>, Bearer>,
    pub spend: HarvestOrderSpend,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BufferingBatch<GaugeId, StateId, Bearer> {
    pub gauges: Vec<Bundled<Gauge<GaugeId, StateId>, Bearer>>,
    pub buffer_wallet: Bundled<BufferWallet<StateId>, Bearer>,
    pub auth_manager: Bundled<AuthManager<GaugeId, StateId>, Bearer>,
}

impl<GaugeId, StateId, Bearer> BufferingBatch<GaugeId, StateId, Bearer> {
    pub fn new(
        buffer_wallet: Bundled<BufferWallet<StateId>, Bearer>,
        auth_manager: Bundled<AuthManager<GaugeId, StateId>, Bearer>,
    ) -> Self {
        Self {
            buffer_wallet,
            auth_manager,
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
