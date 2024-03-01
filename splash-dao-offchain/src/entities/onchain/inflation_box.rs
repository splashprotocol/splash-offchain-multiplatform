use crate::time::ProtocolEpoch;

#[derive(Copy, Clone, Debug)]
pub struct InflationBox {
    pub last_processed_epoch: ProtocolEpoch,
}
