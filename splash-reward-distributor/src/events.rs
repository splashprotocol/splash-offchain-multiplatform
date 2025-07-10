use crate::onchain::buffer_wallet::BufferWallet;
use crate::onchain::smart_farm::SmartFarm;

#[derive(Debug, Clone, PartialEq)]
pub enum OnChainEvent<StateId, Bearer> {
    NewHarvestRequest(NewHarvestRequest),
    HarvestRequestCancelled(HarvestRequestCancelled),
    Harvested(Harvested),
    BufferWalletUpdated(BufferWalletUpdated<StateId, Bearer>),
    GaugeUpdated(GaugeUpdated<StateId, Bearer>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct NewHarvestRequest {
    // Add fields as needed for the new reward withdrawal request event
}

#[derive(Debug, Clone, PartialEq)]
pub struct HarvestRequestCancelled {
    // Add fields as needed for the cancelled withdrawal request event
}

#[derive(Debug, Clone, PartialEq)]
pub struct Harvested {
    // Add fields as needed for the reward withdrawn event
}

#[derive(Debug, Clone, PartialEq)]
pub struct BufferWalletUpdated<StateId, Bearer> {
    wallet_consumed: StateId,
    wallet_created: (BufferWallet<StateId>, Bearer),
}

#[derive(Debug, Clone, PartialEq)]
pub struct GaugeUpdated<StateId, Bearer> {
    farm_consumed: StateId,
    farm_created: (SmartFarm<StateId>, Bearer),
}
