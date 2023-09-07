use std::marker::PhantomData;

use cml_chain::plutus::PlutusData;
use cml_chain::{AssetName, PolicyId};
use cml_crypto::{RawBytesEncoding, TransactionHash};
use derivative::Derivative;

use crate::constants::NATIVE_POLICY_ID;
use crate::plutus_data::{ConstrPlutusDataExtension, PlutusDataExtension};
use crate::types::TryFromPData;

pub mod constants;
pub mod plutus_data;
pub mod types;

pub type OutputRef = (TransactionHash, u64);

pub type Token = (PolicyId, AssetName);

#[derive(Debug, Clone, Eq, PartialEq, Ord, PartialOrd, Hash)]
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
        let asset_name = AssetName::new(cpd.take_field(0)?.into_bytes()?).ok()?;
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

impl<T> TryFromPData for TaggedAmount<T> {
    fn try_from_pd(data: PlutusData) -> Option<Self> {
        Some(Self(data.into_u64()?, PhantomData::default()))
    }
}
