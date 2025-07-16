pub mod buffer_wallet;
pub mod harvest_order;
pub mod smart_farm;

#[repr(u8)]
#[derive(Eq, PartialEq)]
pub enum RewardProtocolValidator {
    WalletBuffer = 150,
    HarvestOrder = 151,
}
