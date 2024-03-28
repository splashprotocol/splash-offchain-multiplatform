use std::cmp::Ordering;
use std::fmt::{Debug, Display, Formatter};
use std::marker::PhantomData;
use std::ops::{Add, AddAssign, Sub, SubAssign};
use std::str::FromStr;

use cml_chain::assets::MultiAsset;
use cml_chain::plutus::PlutusData;
use cml_chain::transaction::TransactionInput;
use cml_chain::{PolicyId, Value};
use cml_crypto::{RawBytesEncoding, TransactionHash};
use derivative::Derivative;
use derive_more::{From, Into};
use serde::Deserialize;

use crate::plutus_data::{ConstrPlutusDataExtension, PlutusDataExtension};
use crate::types::TryFromPData;

pub mod address;
pub mod collateral;
pub mod constants;
pub mod credential;
pub mod hash;
pub mod output;
pub mod plutus_data;
pub mod protocol_params;
pub mod transaction;
pub mod types;
pub mod value;

/// Asset name bytes padded to 32-byte fixed array and tupled with the len of the original asset name.
#[derive(Debug, Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Hash, derive_more::From)]
pub struct AssetName(u8, [u8; 32]);

impl AssetName {
    pub fn padded_bytes(&self) -> [u8; 32] {
        self.1
    }

    pub fn utf8_unsafe(tn: String) -> Self {
        let orig_len = tn.len();
        let tn = if orig_len > 32 { &tn[0..32] } else { &*tn };
        let mut bf = [0u8; 32];
        tn.as_bytes().into_iter().enumerate().for_each(|(ix, i)| {
            bf[ix] = *i;
        });
        Self(orig_len as u8, bf)
    }
}

impl Display for AssetName {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(format!("{}", std::str::from_utf8(&self.1).unwrap()).as_str())
    }
}

impl From<AssetName> for cml_chain::assets::AssetName {
    fn from(AssetName(orig_len, raw_name): AssetName) -> Self {
        cml_chain::assets::AssetName {
            inner: raw_name[0..orig_len as usize].to_vec(),
            encodings: None,
        }
    }
}

#[derive(Debug)]
pub struct AssetNameParsingError;

impl TryFrom<String> for AssetName {
    type Error = AssetNameParsingError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        AssetName::try_from(value.as_bytes().to_vec()).map_err(|_| AssetNameParsingError)
    }
}

impl From<cml_chain::assets::AssetName> for AssetName {
    fn from(value: cml_chain::assets::AssetName) -> Self {
        AssetName::try_from(value.inner).unwrap()
    }
}

#[derive(Debug)]
pub struct InvalidAssetNameError;

impl TryFrom<Vec<u8>> for AssetName {
    type Error = InvalidAssetNameError;
    fn try_from(value: Vec<u8>) -> Result<Self, Self::Error> {
        let orig_len = value.len();
        if orig_len > 32 {
            return Err(InvalidAssetNameError);
        };
        let orig_len = <u8>::try_from(orig_len).unwrap();
        let mut bf = [0u8; 32];
        value.into_iter().enumerate().for_each(|(ix, i)| {
            bf[ix] = i;
        });
        Ok(Self(orig_len, bf))
    }
}

#[derive(Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Deserialize)]
#[serde(try_from = "String")]
pub struct OutputRef(TransactionHash, u64);
impl OutputRef {
    pub fn new(hash: TransactionHash, index: u64) -> Self {
        Self(hash, index)
    }
    pub fn tx_hash(&self) -> TransactionHash {
        self.0
    }
    pub fn index(&self) -> u64 {
        self.1
    }
}

impl Debug for OutputRef {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        Display::fmt(self, f)
    }
}

impl Display for OutputRef {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(format!("{}#{}", self.0.to_hex(), self.1).as_str())
    }
}

impl From<TransactionInput> for OutputRef {
    fn from(value: TransactionInput) -> Self {
        Self(value.transaction_id, value.index)
    }
}

impl From<(TransactionHash, u64)> for OutputRef {
    fn from((h, i): (TransactionHash, u64)) -> Self {
        Self(h, i)
    }
}

impl From<OutputRef> for TransactionInput {
    fn from(OutputRef(hash, ix): OutputRef) -> Self {
        TransactionInput::new(hash, ix)
    }
}

impl TryFrom<String> for OutputRef {
    type Error = &'static str;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        OutputRef::try_from(&*value)
    }
}

impl TryFrom<&str> for OutputRef {
    type Error = &'static str;
    fn try_from(value: &str) -> Result<Self, Self::Error> {
        if let Some((raw_tx_id, str_idx)) = value.split_once("#") {
            return Ok(OutputRef(
                TransactionHash::from_hex(raw_tx_id).unwrap(),
                u64::from_str(str_idx).unwrap(),
            ));
        }
        Err("Invalid OutputRef")
    }
}

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

    pub fn into_value(self, amount: u64) -> Value {
        let mut value = Value::zero();
        match self {
            AssetClass::Native => value.coin += amount,
            AssetClass::Token((policy, an)) => {
                let mut ma = MultiAsset::new();
                ma.set(policy, an.into(), amount);
                value.multiasset = ma;
            }
        }
        value
    }
}

impl Display for AssetClass {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            AssetClass::Native => f.write_str("Native"),
            AssetClass::Token((pol, tn)) => f.write_str(format!("{}:{}", pol, tn).as_str()),
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
        let policy_bytes = cpd.take_field(0)?.into_bytes()?;
        if policy_bytes.is_empty() {
            Some(AssetClass::Native)
        } else {
            let policy_id = PolicyId::from_raw_bytes(&*policy_bytes).ok()?;
            let asset_name = AssetName::try_from(cpd.take_field(1)?.into_bytes()?).ok()?;
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

impl<T> TaggedAssetClass<T> {
    pub fn new(ac: AssetClass) -> Self {
        Self(ac, PhantomData::default())
    }
    pub fn is_native(&self) -> bool {
        matches!(self.0, AssetClass::Native)
    }
    pub fn untag(self) -> AssetClass {
        self.0
    }
}

impl<T> TryFromPData for TaggedAssetClass<T> {
    fn try_from_pd(data: PlutusData) -> Option<Self> {
        Some(Self(AssetClass::try_from_pd(data)?, PhantomData::default()))
    }
}

#[repr(transparent)]
#[derive(Derivative)]
#[derivative(Debug(bound = ""), Copy(bound = ""), Clone(bound = ""), Eq(bound = ""))]
pub struct TaggedAmount<T>(u64, PhantomData<T>);

impl<T> PartialEq for TaggedAmount<T> {
    fn eq(&self, other: &Self) -> bool {
        self.0.eq(&other.0)
    }
}

impl<T> PartialOrd for TaggedAmount<T> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        self.0.partial_cmp(&other.0)
    }
}

impl<T> TaggedAmount<T> {
    pub fn new(value: u64) -> Self {
        Self(value, PhantomData::default())
    }

    pub fn untag(self) -> u64 {
        self.0
    }

    pub fn retag<T1>(self) -> TaggedAmount<T1> {
        TaggedAmount(self.0, PhantomData::default())
    }
}

impl<T> AsRef<u64> for TaggedAmount<T> {
    fn as_ref(&self) -> &u64 {
        &self.0
    }
}

impl<T> AsMut<u64> for TaggedAmount<T> {
    fn as_mut(&mut self) -> &mut u64 {
        &mut self.0
    }
}

impl<T> Add for TaggedAmount<T> {
    type Output = Self;
    fn add(self, rhs: Self) -> Self::Output {
        Self(self.0 + rhs.0, PhantomData::default())
    }
}

impl<T> AddAssign for TaggedAmount<T> {
    fn add_assign(&mut self, rhs: Self) {
        self.0 += rhs.0
    }
}

impl<T> Sub for TaggedAmount<T> {
    type Output = Self;
    fn sub(self, rhs: Self) -> Self::Output {
        Self(self.0 - rhs.0, PhantomData::default())
    }
}

impl<T> SubAssign for TaggedAmount<T> {
    fn sub_assign(&mut self, rhs: Self) {
        self.0 -= rhs.0
    }
}

impl<T> TryFromPData for TaggedAmount<T> {
    fn try_from_pd(data: PlutusData) -> Option<Self> {
        Some(Self(data.into_u64()?, PhantomData::default()))
    }
}

pub type NetworkTime = u64;

#[derive(serde::Deserialize, Debug, Copy, Clone, From, Into)]
pub struct NetworkId(u8);

#[cfg(test)]
mod tests {
    use crate::AssetName;

    #[test]
    fn asset_name_is_isomorphic_to_cml() {
        let len = 14;
        let cml_an = cml_chain::assets::AssetName::new(vec![0u8; len]).unwrap();
        let spectrum_an = AssetName::from(cml_an.clone());
        let cml_an_reconstructed = cml_chain::assets::AssetName::from(spectrum_an);
        assert_eq!(cml_an, cml_an_reconstructed);
    }
}
