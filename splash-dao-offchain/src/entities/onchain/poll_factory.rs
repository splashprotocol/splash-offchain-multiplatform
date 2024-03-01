use crate::time::ProtocolEpoch;
use crate::FarmId;

pub struct PollFactory {
    pub last_poll_epoch: ProtocolEpoch,
    pub active_farms: Vec<FarmId>,
}

impl PollFactory {
    pub fn next_epoch(&self) -> ProtocolEpoch {
        self.last_poll_epoch + 1
    }
}
