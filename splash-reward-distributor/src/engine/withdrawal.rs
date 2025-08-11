use crate::engine::resolved_tx::PartiallySignedCardanoTx;
use crate::onchain::harvest_order::HarvestOrder;
use spectrum_offchain::ledger::TryFromLedger;

#[derive(Debug)]
pub struct Withdrawal<OrderId> {
    // Order that requested harvesting
    pub order: HarvestOrder<OrderId>,
    // Amount withdrawn from buffer wallet
    pub amount: u64,
}

impl<OrderId, Ctx> TryFromLedger<PartiallySignedCardanoTx, Ctx> for Withdrawal<OrderId> {
    fn try_from_ledger(repr: &PartiallySignedCardanoTx, ctx: &Ctx) -> Option<Self> {
        todo!("DEX-906")
    }
}
