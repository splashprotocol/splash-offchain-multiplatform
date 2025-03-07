use crate::account::AccountInPool;
use cml_chain::certs::Credential;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
pub struct ExportAccountEvent {
    pub account_cred: Credential,
    pub update: AccountInPool,
}
