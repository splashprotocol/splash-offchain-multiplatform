use crate::NetworkId;

pub fn slot_to_time_millis(slot: u64, network_id: NetworkId) -> u64 {
    slot_to_posix(slot, network_id) * 1000
}

pub fn slot_to_posix(slot: u64, network_id: NetworkId) -> u64 {
    if network_id == NetworkId::PREPROD {
        // Preprod
        1655683200 + slot
    } else {
        // Mainnet
        1591566291 + slot
    }
}

pub fn posix_to_slot(posix: u64, network_id: NetworkId) -> u64 {
    if network_id == NetworkId::PREPROD {
        posix - 1655683200
    } else {
        posix - 1591566291
    }
}
