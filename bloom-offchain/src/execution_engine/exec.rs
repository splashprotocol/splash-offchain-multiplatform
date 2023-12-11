/// Defines how to execute a particular on-chain action with some entity.
/// Effect of execution are supposed to be accumulated in [BatchAcc] (usually a TX builder).
pub trait BatchExec<BatchAcc, Ctx, Err> {
    fn try_exec(self, accumulator: BatchAcc, context: Ctx) -> Result<BatchAcc, Err>;
}
