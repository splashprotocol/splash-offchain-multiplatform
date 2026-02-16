use cml_crypto::blake2b256;
use serde::{Deserialize, Serialize};
use spectrum_cardano_lib::OutputRef;
use splash_dao_offchain::entities::onchain::smart_farm::FarmId;
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

impl AsRef<[u8]> for TaskId {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

impl TryFrom<Vec<u8>> for TaskId {
    type Error = ();
    fn try_from(value: Vec<u8>) -> Result<Self, Self::Error> {
        <[u8; 32]>::try_from(&*value).map(Self).map_err(|_| ())
    }
}

impl From<FarmId> for TaskId {
    fn from(farm_id: FarmId) -> Self {
        blake2b256(farm_id.0.as_bytes()).into()
    }
}

impl From<OutputRef> for TaskId {
    fn from(oref: OutputRef) -> Self {
        blake2b256(oref.to_string().as_bytes()).into()
    }
}

#[derive(Copy, Clone, Serialize, Deserialize)]
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

#[derive(Copy, Clone, Serialize, Deserialize)]
pub struct GaugeBuffering<GaugeId> {
    pub gauge_id: GaugeId,
}

#[derive(Copy, Clone, Serialize, Deserialize)]
pub struct Harvesting<OrderId> {
    pub order_id: OrderId,
}
