#[derive(Debug, Copy, Clone)]
pub struct ExecutionConfig<U> {
    pub ex_limits: ExecutionLimits<U>,
    /// Order-order matchmaking allowed.
    pub o2o_allowed: bool,
}

#[derive(Debug, Copy, Clone)]
pub struct ExecutionCap<U> {
    pub soft: U,
    pub hard: U,
}

#[derive(Debug, Copy, Clone)]
pub struct ExecutionLimits<U> {
    pub ex_units: ExecutionCap<U>,
    pub max_trades_in_batch: usize,
}
