use crate::Epoch;

#[derive(Copy, Clone, Debug)]
pub struct InflationBox {
    pub last_processed_epoch: Epoch,
}
