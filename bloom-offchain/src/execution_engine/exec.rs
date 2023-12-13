/// Defines how to execute a particular on-chain action with some entity.
/// Effect of execution are supposed to be accumulated in [BatchAcc] (usually a TX builder).
/// Execution possible results into a [Next] state of the entity.
pub trait BatchExec<BatchAcc, Next, Ctx, Err> {
    fn try_exec(self, accumulator: BatchAcc, context: Ctx) -> Result<(BatchAcc, Next), Err>;
}
