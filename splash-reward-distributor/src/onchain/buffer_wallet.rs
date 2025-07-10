#[derive(Debug, Copy, Clone, PartialEq)]
pub struct WalletId(u64);

#[derive(Debug, Clone, PartialEq)]
pub struct BufferWallet<StateId> {
    state_id: StateId,
}