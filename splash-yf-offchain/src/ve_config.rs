use cml_core::Slot;

#[derive(Debug, Copy, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VeConfig {
    pub epoch_start: Slot,
    pub slots_in_epoch: u64,
}
