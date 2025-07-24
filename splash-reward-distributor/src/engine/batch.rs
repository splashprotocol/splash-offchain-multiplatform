use crate::onchain::buffer_wallet::BufferWallet;
use crate::onchain::harvest_order::HarvestOrder;
use crate::onchain::smart_farm::Gauge;
use bloom_offchain::execution_engine::bundled::Bundled;
use cml_chain::certs::Credential;

#[derive(Debug, Clone, PartialEq)]
pub struct HarvestBatch<StateId, Bearer> {
    buffer_wallet: Bundled<BufferWallet<StateId>, Bearer>,
    orders: Vec<Bundled<HarvestOrder<StateId>, Bearer>>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum HarvestBatchRejection {
    MaxCapacityReached,
}

impl<StateId, Bearer> HarvestBatch<StateId, Bearer> {
    pub fn new(
        buffer_wallet: Bundled<BufferWallet<StateId>, Bearer>,
        order: Bundled<HarvestOrder<StateId>, Bearer>,
    ) -> Self {
        Self {
            buffer_wallet,
            orders: vec![order],
        }
    }

    pub fn orders(&self) -> Vec<&HarvestOrder<StateId>> {
        self.orders.iter().map(|Bundled(t, _)| t).collect()
    }

    pub fn try_add_order(
        &mut self,
        order: Bundled<HarvestOrder<StateId>, Bearer>,
    ) -> Result<(), HarvestBatchRejection> {
        self.orders.push(order);
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct BufferingBatch<GaugeId, StateId, Bearer> {
    pub gauges: Vec<Bundled<Gauge<GaugeId, StateId>, Bearer>>,
    pub buffer_wallet: Bundled<BufferWallet<StateId>, Bearer>,
}
