use crate::entities::offchain::voting_order::VotingOrderId;
use crate::FarmId;

pub enum InflationEffect<TxId> {
    AttemptCreatePoll(Option<TxId>),
    AttemptEliminatePoll(Option<TxId>),
    AttemptOrderExec(Result<VotingOrderId, VotingOrderId>),
    InflationDistributedTo(FarmId),
}
