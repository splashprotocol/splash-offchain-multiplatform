use cml_chain::address::Address;
use cml_chain::builders::input_builder::SingleInputBuilder;
use cml_chain::certs::StakeCredential;
use cml_chain::transaction::TransactionOutput;
use cml_crypto::ScriptHash;
use serde::{Deserialize, Serialize};
use spectrum_cardano_lib::OutputRef;
use spectrum_offchain::domain::Has;
use spectrum_offchain::ledger::{IntoLedger, TryFromLedger};
use spectrum_offchain_cardano::data::pair::PairId;
use spectrum_offchain_cardano::deployment::DeployedScriptInfo;
use std::collections::HashMap;
use spectrum_cardano_lib::transaction::TransactionOutputExtension;
use splash_dao_offchain::entities::HasStatus;

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub struct BufferedWallet {
    pub id: OutputRef,
    pub status: BufferedWalletStatus,
    // todo: use value
    pub splash_amount: u64,
    pub lovelace_amount: u64,
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub enum BufferedWalletStatus {
    Free,
    SFWithdrawInProgress,
    UserWithdraw,
}

impl HasStatus for BufferedWallet {
    type Status = BufferedWalletStatus;

    fn get_status(self) -> Self::Status {
        self.status
    }
}

impl<Ctx> IntoLedger<SingleInputBuilder, Ctx> for BufferedWallet {
    fn into_ledger(self, ctx: Ctx) -> SingleInputBuilder {
        todo!()
    }
}
