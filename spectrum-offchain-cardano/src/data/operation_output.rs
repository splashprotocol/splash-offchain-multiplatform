use cml_chain::transaction::TransactionOutput;
use cml_chain::Coin;
use cml_crypto::Ed25519KeyHash;

use spectrum_cardano_lib::{TaggedAmount, TaggedAssetClass};
use spectrum_offchain::ledger::IntoLedger;

use crate::data::order::Quote;

#[derive(Debug, Clone)]
pub struct SwapOutput {
    pub quote_asset: TaggedAssetClass<Quote>,
    pub quote_amount: TaggedAmount<Quote>,
    pub ada_residue: Coin,
    pub redeemer_pkh: Ed25519KeyHash,
}

impl IntoLedger<TransactionOutput> for SwapOutput {
    fn into_ledger(self) -> TransactionOutput {
        todo!()
    }
}
