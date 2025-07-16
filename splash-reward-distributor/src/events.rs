use cml_crypto::Ed25519KeyHash;
use splash_dao_offchain::entities::onchain::{permission_manager::PermManager, smart_farm::SmartFarm};

use crate::onchain::{buffer_wallet::BufferWallet, harvest_order::HarvestOrderSnapshot};

#[derive(Debug, Clone, PartialEq)]
pub enum OnChainEvent<Bearer> {
    NewHarvestRequest(HarvestOrderSnapshot),
    HarvestRequestCancelled(HarvestRequestCancelled<Bearer>),
    Harvested(Harvested<Bearer>),
    BufferWalletUpdated(BufferWalletUpdated<Bearer>),
    GaugeUpdated(GaugeUpdated<Bearer>),
    PermManagerUpdated(PermManagerUpdated<Bearer>),
}

#[derive(Debug, Clone, PartialEq)]
pub struct HarvestRequestCancelled<Bearer> {
    user_key_hash: Ed25519KeyHash,
    bearer: Bearer,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Harvested<Bearer> {
    user_key_hash: Ed25519KeyHash,
    reward_amount: u64,
    bearer: Bearer,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BufferWalletUpdated<Bearer> {
    wallet_consumed: Bearer,
    wallet_created: (BufferWallet, Bearer),
}

#[derive(Debug, Clone, PartialEq)]
pub struct GaugeUpdated<Bearer> {
    farm_consumed: Bearer,
    farm_created: (SmartFarm, Bearer),
}

#[derive(Debug, Clone, PartialEq)]
pub struct PermManagerUpdated<Bearer> {
    pm_consumed: Bearer,
    pm_created: (PermManager, Bearer),
}
