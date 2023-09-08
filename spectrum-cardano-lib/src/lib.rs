use std::array::TryFromSliceError;
use std::marker::PhantomData;

use cml_chain::plutus::PlutusData;
use cml_chain::PolicyId;
use cml_crypto::{RawBytesEncoding, TransactionHash};
use derivative::Derivative;

use crate::constants::NATIVE_POLICY_ID;
use crate::plutus_data::{ConstrPlutusDataExtension, PlutusDataExtension};
use crate::types::TryFromPData;

pub mod constants;
pub mod plutus_data;
pub mod transaction;
pub mod types;
pub mod value;

#[repr(transparent)]
#[derive(Debug, Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct AssetName([u8; 32]);

impl From<AssetName> for cml_chain::AssetName {
    fn from(value: AssetName) -> Self {
        cml_chain::AssetName {
            inner: value.0.to_vec(),
            encodings: None,
        }
    }
}

impl From<cml_chain::AssetName> for AssetName {
    fn from(value: cml_chain::AssetName) -> Self {
        Self(<[u8; 32]>::try_from(value.inner).unwrap())
    }
}

impl TryFrom<Vec<u8>> for AssetName {
    type Error = TryFromSliceError;
    fn try_from(value: Vec<u8>) -> Result<Self, Self::Error> {
        Ok(Self(<[u8; 32]>::try_from(&*value)?))
    }
}

pub type OutputRef = (TransactionHash, u64);

pub type Token = (PolicyId, AssetName);

#[derive(Debug, Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub enum AssetClass {
    Native,
    Token(Token),
}

impl AssetClass {
    pub fn into_token(self) -> Option<Token> {
        match self {
            AssetClass::Token(tkn) => Some(tkn),
            AssetClass::Native => None,
        }
    }
}

impl<T> From<TaggedAssetClass<T>> for AssetClass {
    fn from(value: TaggedAssetClass<T>) -> Self {
        value.0
    }
}

impl TryFromPData for AssetClass {
    fn try_from_pd(data: PlutusData) -> Option<Self> {
        let mut cpd = data.into_constr_pd()?;
        let policy_id = PolicyId::from_raw_bytes(&*cpd.take_field(0)?.into_bytes()?).ok()?;
        let asset_name = AssetName::try_from(cpd.take_field(0)?.into_bytes()?).ok()?;
        if policy_id == *NATIVE_POLICY_ID {
            Some(AssetClass::Native)
        } else {
            Some(AssetClass::Token((policy_id, asset_name)))
        }
    }
}

#[repr(transparent)]
#[derive(Derivative)]
#[derivative(
    Debug(bound = ""),
    Copy(bound = ""),
    Clone(bound = ""),
    Eq(bound = ""),
    PartialEq(bound = ""),
    Ord(bound = ""),
    PartialOrd(bound = ""),
    Hash(bound = "")
)]
pub struct TaggedAssetClass<T>(AssetClass, PhantomData<T>);

impl<T> TryFromPData for TaggedAssetClass<T> {
    fn try_from_pd(data: PlutusData) -> Option<Self> {
        Some(Self(AssetClass::try_from_pd(data)?, PhantomData::default()))
    }
}

#[repr(transparent)]
#[derive(Derivative)]
#[derivative(Debug(bound = ""), Copy(bound = ""), Clone(bound = ""))]
pub struct TaggedAmount<T>(u64, PhantomData<T>);

impl<T> TaggedAmount<T> {
    pub fn unsafe_tag(value: u64) -> Self {
        Self(value, PhantomData::default())
    }
}

impl<T> TryFromPData for TaggedAmount<T> {
    fn try_from_pd(data: PlutusData) -> Option<Self> {
        Some(Self(data.into_u64()?, PhantomData::default()))
    }
}
