use cml_chain::certs::Credential;
use cml_crypto::{Ed25519KeyHash, ScriptHash};
use either::Either;

#[derive(Copy, Clone, Eq, PartialEq, Debug)]
pub struct AnyCredential(Either<Ed25519KeyHash, ScriptHash>);

impl From<Credential> for AnyCredential {
    fn from(value: Credential) -> Self {
        Self(match value {
            Credential::PubKey { hash, .. } => Either::Left(hash),
            Credential::Script { hash, .. } => Either::Right(hash),
        })
    }
}

impl From<AnyCredential> for Credential {
    fn from(value: AnyCredential) -> Self {
        match value.0 {
            Either::Left(hash) => Credential::PubKey {
                hash,
                len_encoding: Default::default(),
                tag_encoding: None,
                hash_encoding: Default::default(),
            },
            Either::Right(hash) => Credential::Script {
                hash,
                len_encoding: Default::default(),
                tag_encoding: None,
                hash_encoding: Default::default(),
            },
        }
    }
}
