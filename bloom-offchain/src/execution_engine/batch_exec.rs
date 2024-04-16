/// Defines how to execute a particular on-chain action with some entity.
/// Effect of execution are supposed to be accumulated in [BatchAcc] (usually a TX builder).
/// Execution results into a successive state [Succ] of the entity.
pub trait BatchExec<BatchAcc, Succ, Ctx> {
    fn exec(self, accumulator: BatchAcc, context: Ctx) -> (BatchAcc, Succ, Ctx);
}
