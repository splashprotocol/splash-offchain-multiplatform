use crate::network_time::NetworkTime;

#[derive(Copy, Clone, Debug)]
pub struct CoinSeedConfig {
    pub zeroth_epoch_start: NetworkTime,
}
