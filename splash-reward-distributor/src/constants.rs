pub const HARVESTING_TX_FEE_DELTA: u64 = 400_000;
pub const HARVESTING_TX_ASSUMED_BASE_FEE: u64 = 800_000;
/// Extra ADA to add buffer_wallt output for harvesting TX (CML's set_min_ada(...) doesn't add
/// enough ADA)
pub const BUFFER_WALLET_ADA_BUFFER: u64 = 10_000;
pub const GAUGE_BUFFERING_TX_FEE_DELTA: u64 = 400_000;
pub const GAUGE_BUFFERING_TX_MINIMAL_FUNDING_BOX_BALANCE: u64 = 2_000_000;
