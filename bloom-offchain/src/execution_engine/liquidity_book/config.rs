#[derive(Debug, Copy, Clone)]
pub struct ExecutionConfig<U> {
    pub execution_cap: ExecutionCap<U>,
    /// Order-order matchmaking allowed.
    pub o2o_allowed: bool,
}

#[derive(Debug, Copy, Clone)]
pub struct ExecutionCap<U> {
    pub soft: U,
    pub hard: U,
}
