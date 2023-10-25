use cml_chain::address::Address;
use cml_chain::certs::StakeCredential;
use cml_crypto::ScriptHash;

pub trait AddressExtension {
    fn script_hash(&self) -> Option<ScriptHash>;
}

impl AddressExtension for Address {
    fn script_hash(&self) -> Option<ScriptHash> {
        match self.payment_cred()? {
            StakeCredential::PubKey { .. } => None,
            StakeCredential::Script { hash, .. } => Some(*hash),
        }
    }
}
