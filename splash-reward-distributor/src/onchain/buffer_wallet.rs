#[derive(Debug, Clone, PartialEq)]
pub struct BufferWallet<StateId> {
    pub state_id: StateId,
    pub balance: u64,
}
