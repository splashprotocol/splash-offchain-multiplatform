use crate::execution_engine::liquidity_book::core::BaseStepBudget;
use spectrum_offchain::domain::Has;
use type_equalities::IsEqual;

#[derive(Debug, Copy, Clone)]
pub struct ExecutionConfig<U> {
    pub execution_cap: ExecutionCap<U>,
    /// Order-order matchmaking allowed.
    pub o2o_allowed: bool,
    pub base_step_budget: BaseStepBudget,
}

impl<U> Has<BaseStepBudget> for ExecutionConfig<U> {
    fn select<K: IsEqual<BaseStepBudget>>(&self) -> BaseStepBudget {
        self.base_step_budget
    }
}

#[derive(Debug, Copy, Clone)]
pub struct ExecutionCap<U> {
    pub soft: U,
    pub hard: U,
}
