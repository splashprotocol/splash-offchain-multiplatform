pub const SPLASH_NAME: &str = "SPLASH";

pub const GT_NAME: u8 = 244;

pub const LQ_NAME: &str = "SPLASH/ADA LQ*";

pub const DEFAULT_AUTH_TOKEN_NAME: u8 = 164;

/// Length of one emission epoch in milliseconds.
pub const EPOCH_LEN: u64 = 604_800_000;

/// Length of the emission reduction period in epochs.
pub const EMISSION_REDUCTION_PERIOD_LEN: u64 = 13;

/// MAX supply of Gov token (== MAX supply of SPLASH/ADA LQ* == MAX supply of ADA lovelace).
pub const MAX_GT_SUPPLY: u64 = 45_000_000_000_000_000;

pub const TOTAL_EMISSION: u64 = 100_000_000_000_000;

pub const TOTAL_COMMUNITY_EMISSION: u64 = 32_000_000_000_000;

/// Epochly inflation rate in micro-SPLASH.
pub const RATE_INITIAL: u64 = 224_000_000_000;

pub const RATE_AFTER_FIRST_REDUCTION: u64 = 156_800_000_000;

/// Reduction rate applied after first reduction.
pub const TAIL_REDUCTION_RATE_NUM: u64 = 94_524;

pub const TAIL_REDUCTION_RATE_DEN: u64 = 100_000;

/// Maximum tolerable time inaccuracy. (12 hours)
pub const MAX_TIME_DRIFT_MILLIS: u64 = 43_200_000;

/// Max lock timespan in seconds. (1 year)
pub const MAX_LOCK_TIME_SECONDS: u64 = 31_536_000;

pub const MAX_LOCK_TIME_MILLIS: u64 = 31_536_000_000;

/// Max length of voting on proposal. (30 days)
pub const MAX_VOTING_TIME_MILLIS: u64 = 2_592_000_000;

/// Min length of voting on proposal. (7 days)
pub const MIN_VOTING_TIME_MILLIS: u64 = 604_800_000;

pub const MIN_PROPOSAL_OPTIONS: usize = 2;

pub const MILLIS_IN_SECOND: u64 = 1000;

/// Constant index of a proposal (GovProposal or WeightingPoll) output.
pub const PROPOSAL_OUT_INDEX: usize = 0;

/// Constant index of Voting Escrow output in gov action.
pub const VE_OUT_INDEX_NON_GOV: usize = 0;

/// Constant index of Voting Escrow output in non-gov actions.
pub const VE_OUT_INDEX_GOV: usize = 1;

/// Constant index of Smart Farm output.
pub const FARM_OUT_INDEX: usize = 1;
