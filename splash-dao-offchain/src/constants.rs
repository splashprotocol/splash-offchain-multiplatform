use once_cell::sync::OnceCell;

use crate::deployment::DaoScriptData;

pub const SPLASH_NAME: &str = "SPLASH";

pub const GT_NAME: u8 = 244;

pub const LQ_NAME: &str = "SPLASH/ADA LQ*";

pub const DEFAULT_AUTH_TOKEN_NAME: u8 = 164;

/// Length of the emission reduction period in epochs.
pub const EMISSION_REDUCTION_PERIOD_LEN: u32 = 13;

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

#[cfg(not(feature = "test_30_min_epoch"))]
pub mod time {
    /// Length of one emission epoch in milliseconds.
    pub const EPOCH_LEN: u64 = 604_800_000;

    /// Maximum tolerable time inaccuracy. (12 hours)
    pub const MAX_TIME_DRIFT_MILLIS: u64 = 43_200_000;

    /// Max lock timespan in seconds. (1 year)
    pub const MAX_LOCK_TIME_SECONDS: u64 = 31_536_000;

    /// Max length of voting on proposal. (30 days)
    pub const MAX_VOTING_TIME_MILLIS: u64 = 2_592_000_000;

    /// Min length of voting on proposal. (7 days)
    pub const MIN_VOTING_TIME_MILLIS: u64 = 604_800_000;

    /// Period after poll deadline after which it is allowed to destroy the poll. 30 min.
    pub const COOLDOWN_PERIOD_MILLIS: u64 = 1_800_000;

    /// Extra buffer period after COOLDOWN_PERIOD_MILLIS to ensure poll elimination TX validates. 5 min.
    pub const COOLDOWN_PERIOD_EXTRA_BUFFER: u64 = 300_000;

    /// Period after poll deadline after which it is allowed to start distributing inflation. 10 min.
    pub const DISTRIBUTE_INFLATION_START_DELAY_MILLIS: u64 = 600_000;

    /// This constant represents the number of milliseconds to add on top of the POSIX time of the
    /// right-side epoch boundary. This is used to ensure that a TX's validity interval is entirely
    /// within the next epoch. 1 hour.
    pub const EPOCH_BOUNDARY_SHIFT: u64 = 3_600_000;

    /// TX time-to-live (TTL) for `distribute_inflation` TX (# slots).
    pub const DISTRIBUTE_INFLATION_TX_TTL: u64 = 300;
}

pub mod fee_deltas {
    pub const CREATE_WPOLL_FEE_DELTA: u64 = 200_000;
    pub const ELIMINATE_WPOLL_FEE_DELTA: u64 = 50_000;
    pub const DISTRIBUTE_INFLATION_FEE_DELTA: u64 = 10_000;
    pub const VOTING_ESCROW_VOTING_FEE: u64 = 1_300_000;
}

#[cfg(feature = "test_30_min_epoch")]
pub mod time {
    /// Length of one emission epoch in milliseconds (30 min.).
    pub const EPOCH_LEN: u64 = 1_800_000;

    /// Maximum tolerable time inaccuracy. (10 min.)
    pub const MAX_TIME_DRIFT_MILLIS: u64 = 600_000;

    /// Max lock timespan in seconds. (30 min. * 52 == 1560 min.)
    pub const MAX_LOCK_TIME_SECONDS: u64 = 93_600;

    /// Max length of voting on proposal. (2 hours)
    pub const MAX_VOTING_TIME_MILLIS: u64 = 14_400_000;

    /// Min length of voting on proposal. (5 min)
    pub const MIN_VOTING_TIME_MILLIS: u64 = 300_000;

    /// Period after poll deadline after which it is allowed to destroy the poll. 10 min.
    pub const COOLDOWN_PERIOD_MILLIS: u64 = 600_000;

    /// Extra buffer period after COOLDOWN_PERIOD_MILLIS to ensure poll elimination TX validates. 2 min.
    pub const COOLDOWN_PERIOD_EXTRA_BUFFER: u64 = 120_000;

    /// Period after poll deadline after which it is allowed to start distributing inflation. 2 min.
    pub const DISTRIBUTE_INFLATION_START_DELAY_MILLIS: u64 = 120_000;

    /// This constant represents the number of milliseconds to add on top of the POSIX time of the
    /// right-side epoch boundary. This is used to ensure that a TX's validity interval is entirely
    /// within the next epoch. 2 min.
    pub const EPOCH_BOUNDARY_SHIFT: u64 = 120_000;

    /// TX time-to-live (TTL) for `distribute_inflation` TX (# slots).
    pub const DISTRIBUTE_INFLATION_TX_TTL: u64 = 300;

    //----------------------------------------------------------------------------------------------
    // NOTE: the following constants from governance/weighting_poll.ak were also modified:
    //
    // Period after poll deadline after which it is allowed to destroy the poll. 30 days.
    //   const cooldown_period_millis = 1_800_000
    //   const wp_preinit_period_millis = 600_000 // 10 min.
}

pub const MIN_PROPOSAL_OPTIONS: usize = 2;

/// Constant index of a proposal (GovProposal or WeightingPoll) output.
pub const PROPOSAL_OUT_INDEX: usize = 0;

/// Constant index of Voting Escrow output in gov action.
pub const VE_OUT_INDEX_NON_GOV: usize = 0;

/// Constant index of Voting Escrow output in non-gov actions.
pub const VE_OUT_INDEX_GOV: usize = 1;

/// Constant index of Smart Farm output.
pub const FARM_OUT_INDEX: usize = 1;

/// Constant index of the weighting poll.
pub const WP_OUT_IX: usize = 1;

pub const MAKE_VOTING_ESCROW_ORDER_MIN_LOVELACES: u64 = 10_000_000;

pub static DAO_SCRIPT_BYTES: OnceCell<DaoScriptData> = OnceCell::new();
