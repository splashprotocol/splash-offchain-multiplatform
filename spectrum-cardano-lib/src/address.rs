use cml_chain::address::Address;
use cml_chain::certs::{Credential, StakeCredential};
use cml_crypto::ScriptHash;

pub trait AddressExtension {
    fn script_hash(&self) -> Option<ScriptHash>;
    fn update_payment_cred(&mut self, cred: Credential);
}

impl AddressExtension for Address {
    fn script_hash(&self) -> Option<ScriptHash> {
        match self.payment_cred()? {
            StakeCredential::PubKey { .. } => None,
            StakeCredential::Script { hash, .. } => Some(*hash),
        }
    }
    fn update_payment_cred(&mut self, cred: Credential) {
        match self {
            Self::Base(ref mut a) => {
                a.payment = cred;
            }
            Self::Enterprise(ref mut a) => {
                a.payment = cred;
            }
            Self::Ptr(ref mut a) => {
                a.payment = cred;
            }
            Self::Reward(ref mut a) => {
                a.payment = cred;
            }
            Self::Byron(_) => {}
        }
    }
}
