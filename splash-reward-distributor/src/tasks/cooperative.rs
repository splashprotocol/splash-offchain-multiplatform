pub struct CooperativeTask<Tx, Flow> {
    state: TaskState<Tx>,
    flow: Flow,
}

pub enum TaskState<Tx> {
    Planned,
    WaitingCooperator {
        local_attempt_tx: Tx,
    },
    ReadyForSubmission {
        local_attempt_tx: Tx,
        cooperative_attempt_tx: Tx,
    },
    Done { settled_at: Option<u64> },
}