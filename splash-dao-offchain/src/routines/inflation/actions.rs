use bloom_offchain::execution_engine::bundled::Bundled;

use crate::entities::poll_factory::PollFactory;
use crate::time::ProtocolEpoch;

#[async_trait::async_trait]
pub trait InflationActions<Out, TxId> {
    async fn create_wpoll(&self, factory: Bundled<PollFactory, Out>, epoch: ProtocolEpoch) -> TxId;
}

// pub async fn create_wpoll<T, Out, Tx>(
//     Bundled(factory, factory_out): Bundled<PollFactory, Out>,
//     epoch: ProtocolEpoch,
//     ntp: &T,
// ) -> Tx {
//     let new_poll = WeightingPoll::new(epoch, factory.active_farms);
//     todo!()
// }
