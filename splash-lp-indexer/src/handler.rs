use cardano_chain_sync::data::LedgerTxEvent;
use futures::{Sink, SinkExt};
use spectrum_offchain::event_sink::event_handler::EventHandler;
use spectrum_offchain::ledger::TryFromLedger;

pub struct MacroEventHandler<Topic, Cx> {
    topic: Topic,
    context: Cx,
}

#[async_trait::async_trait]
impl<Tx, Topic, Out, Cx> EventHandler<LedgerTxEvent<Tx>> for MacroEventHandler<Topic, Cx>
where
    Topic: Sink<Result<Out, Out>> + Unpin + Send,
    Out: TryFromLedger<Tx, Cx> + Send,
{
    async fn try_handle(&mut self, ev: LedgerTxEvent<Tx>) -> Option<LedgerTxEvent<Tx>> {
        let res = match ev {
            LedgerTxEvent::TxApplied { ref tx, .. } => Out::try_from_ledger(tx, &self.context).map(Ok),
            LedgerTxEvent::TxUnapplied { ref tx, .. } => Out::try_from_ledger(tx, &self.context).map(Err),
        };
        if let Some(event) = res {
            self.topic.send(event).await.unwrap();
        }
        Some(ev)
    }
}
