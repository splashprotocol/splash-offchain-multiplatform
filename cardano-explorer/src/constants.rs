use cml_chain::genesis::network_info::NetworkInfo;
use cml_core::network::BYRON_MAINNET_NETWORK_MAGIC;
use std::string::ToString;

pub const MAINNET_PREFIX: &str = "mainnet";
pub const PREPROD_PREFIX: &str = "preprod";

pub fn get_network_prefix<'a>(network_magic: u64) -> &'a str {
    if network_magic == (u32::from(NetworkInfo::mainnet().protocol_magic()) as u64) {
        MAINNET_PREFIX
    } else {
        PREPROD_PREFIX
    }
}

pub fn get_network_id(network_magic: u64) -> u8 {
    if network_magic == (u32::from(NetworkInfo::mainnet().protocol_magic()) as u64) {
        NetworkInfo::mainnet().network_id()
    } else {
        NetworkInfo::testnet().network_id()
    }
}
