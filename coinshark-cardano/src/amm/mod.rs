use std::{fmt::{Display, Formatter}, str::from_utf8};

use cml_chain::PolicyId;
use cml_crypto::RawBytesEncoding;
use derive_more::Display;

use spectrum_cardano_lib::{OutputRef, Token};

pub mod event_handlers;
pub mod minswap;
pub mod pool;

#[repr(transparent)]
#[derive(Debug, Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Hash, derive_more::From, derive_more::Into)]
pub struct PoolId(Token);

impl Display for PoolId {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        let cml_an = cml_chain::assets::AssetName::from(self.0.1);
        let an = if let Ok(an) = from_utf8(cml_an.get()) {
            String::from(an)
        } else {
            hex::encode(cml_an.inner)
        };
        f.write_str(format!("{}.{}", self.0 .0.to_hex(), an).as_str())
    }
}

impl Into<[u8; 60]> for PoolId {
    fn into(self) -> [u8; 60] {
        let mut bf = [0u8; 60];
        let (policy, an) = self.0;
        policy.to_raw_bytes().into_iter().enumerate().for_each(|(ix, i)| {
            bf[ix] = *i;
        });
        an.padded_bytes().into_iter().enumerate().for_each(|(ix, i)| {
            bf[ix + PolicyId::BYTE_COUNT] = i;
        });
        bf
    }
}

#[repr(transparent)]
#[derive(Debug, Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Hash, derive_more::From, derive_more::Into)]
pub struct PoolVersion(OutputRef);

#[derive(Debug, Copy, Clone, Ord, PartialOrd, Eq, PartialEq, Display)]
pub enum Platform {
    Minswap,
}
