use cml_chain::certs::Credential;

pub struct AccountEvent {
    account_key: Credential,
    update: AccountUpdate,
}

pub enum AccountUpdate {}
