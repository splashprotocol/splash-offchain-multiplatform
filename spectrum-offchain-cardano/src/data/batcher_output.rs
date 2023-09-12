use cml_chain::transaction::TransactionOutput;
use cml_chain::Coin;

use spectrum_offchain::ledger::IntoLedger;

#[derive(Debug, Clone)]
pub struct BatcherProfit {
    pub ada_profit: Coin,
}

impl BatcherProfit {
    pub fn of(coin: Coin) -> Self {
        Self { ada_profit: coin }
    }
}

impl IntoLedger<TransactionOutput> for BatcherProfit {
    fn into_ledger(self) -> TransactionOutput {
        todo!()
    }
}
