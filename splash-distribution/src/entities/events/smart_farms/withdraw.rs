use cml_chain::Value;

#[derive(Clone)]
pub struct SmartFarmWithdraw {
    pub status: SmartFarmWithdrawStatus,
    pub value: Value
}

#[derive(Clone)]
pub enum SmartFarmWithdrawStatus {
    New,
    InProgress,
}