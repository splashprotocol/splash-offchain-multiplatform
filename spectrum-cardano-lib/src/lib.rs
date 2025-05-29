use std::cmp::Ordering;
use std::fmt::{Debug, Display, Formatter};
use std::marker::PhantomData;
use std::ops::{Add, AddAssign, Sub, SubAssign};
use std::str::FromStr;

use crate::constants::{CURRENCY_SYMBOL_HEX_STRING_LENGTH, ED25519_PUB_KEY_LENGTH};
use cml_chain::assets::MultiAsset;
use cml_chain::plutus::utils::ConstrPlutusDataEncoding;
use cml_chain::plutus::{ConstrPlutusData, PlutusData};
use cml_chain::transaction::TransactionInput;
use cml_chain::utils::BigInteger;
use cml_chain::{PolicyId, Value};
use cml_core::serialization::LenEncoding::Indefinite;
use cml_core::DeserializeError;
use cml_crypto::{PublicKey, RawBytesEncoding, TransactionHash};
use derivative::Derivative;
use derive_more::{From, Into};
use num::{CheckedAdd, CheckedSub};
use plutus_data::make_constr_pd_indefinite_arr;
use serde::{Deserialize, Serialize};
use serde_with::SerializeDisplay;

use crate::plutus_data::{ConstrPlutusDataExtension, IntoPlutusData, PlutusDataExtension};
use crate::types::TryFromPData;

pub mod address;
pub mod asset_bundle;
pub mod collateral;
pub mod constants;
pub mod credential;
pub mod ex_units;
pub mod funding;
pub mod hash;
pub mod output;
pub mod plutus_data;
pub mod protocol_params;
pub mod time;
pub mod transaction;
pub mod types;
pub mod value;

/// Asset name bytes padded to 32-byte fixed array and tupled with the len of the original asset name.
#[derive(Debug, Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Hash, derive_more::From)]
pub struct AssetName(u8, [u8; 32]);

impl AssetName {
    pub const fn zero() -> Self {
        AssetName(1, [0u8; 32])
    }

    pub fn padded_bytes(&self) -> [u8; 32] {
        self.1
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.1[0..self.0 as usize]
    }

    pub fn try_from_hex(s: &str) -> Option<Self> {
        hex::decode(s).ok().and_then(|xs| Self::try_from(xs).ok())
    }

    pub fn from_utf8(tn: String) -> Self {
        let orig_len = tn.len();
        let tn = if orig_len > 32 { &tn[0..32] } else { &*tn };
        let mut bf = [0u8; 32];
        tn.as_bytes().into_iter().enumerate().for_each(|(ix, i)| {
            bf[ix] = *i;
        });
        Self(orig_len as u8, bf)
    }
}

impl serde::Serialize for AssetName {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> serde::de::Deserialize<'de> for AssetName {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::de::Deserializer<'de>,
    {
        let s = <String as serde::de::Deserialize>::deserialize(deserializer)?;
        Ok(AssetName::try_from_hex(&s).unwrap_or_else(|| Self::from_utf8(s.clone())))
    }
}

impl Display for AssetName {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        let slice = &self.1[0..self.0 as usize];
        if let Ok(str) = std::str::from_utf8(slice) {
            f.write_str(str)
        } else {
            let as_hex = hex::encode(slice);
            f.write_str(as_hex.as_str())
        }
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

#[derive(Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Deserialize, SerializeDisplay)]
#[serde(try_from = "String", into = "String")]
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

    pub fn from_string_unsafe(s: &str) -> OutputRef {
        Self::try_from(s).unwrap()
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

impl IntoPlutusData for OutputRef {
    fn into_pd(self) -> PlutusData {
        // Note the type for TransactionId in Aiken's stdlib 1.8.0 is different to newer versions.
        // See: https://github.com/aiken-lang/stdlib/blob/c074d343e869b380861b0fc834944c3cefbca982/lib/aiken/transaction.ak#L110
        let transaction_id =
            make_constr_pd_indefinite_arr(vec![PlutusData::new_bytes(self.0.to_raw_bytes().to_vec())]);
        let index = PlutusData::new_integer(BigInteger::from(self.1));
        make_constr_pd_indefinite_arr(vec![transaction_id, index])
    }
}

impl From<OutputRef> for String {
    fn from(value: OutputRef) -> Self {
        format!("{}#{}", value.0.to_hex(), value.1)
    }
}

#[derive(Debug, Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Serialize, Deserialize)]
pub struct Token(pub PolicyId, pub AssetName);

impl Token {
    pub const BYTE_COUNT: usize = 60;

    pub fn try_from_string(s: &str) -> Option<Token> {
        let (pol, an) = s.split_once(".")?;
        Some(Self(PolicyId::from_hex(pol).ok()?, AssetName::try_from_hex(an)?))
    }

    pub fn from_string_unsafe(s: &str) -> Token {
        let parts = s.split(".").collect::<Vec<_>>();
        Self(
            PolicyId::from_hex(parts[0]).unwrap(),
            AssetName::try_from_hex(parts[1]).unwrap(),
        )
    }

    pub fn try_from_raw_string(s: &str) -> Option<Token> {
        let (pol, an) = s.split_at(CURRENCY_SYMBOL_HEX_STRING_LENGTH);
        Some(Self(PolicyId::from_hex(pol).ok()?, AssetName::try_from_hex(an)?))
    }
}

impl TryFrom<String> for Token {
    type Error = &'static str;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::try_from_string(&*value).ok_or("invalid token")
    }
}

impl From<Token> for [u8; Token::BYTE_COUNT] {
    fn from(value: Token) -> Self {
        let mut arr = [0u8; Token::BYTE_COUNT];
        for (ix, b) in value
            .0
            .to_raw_bytes()
            .iter()
            .chain(value.1.padded_bytes().as_slice())
            .enumerate()
        {
            arr[ix] = *b;
        }
        arr
    }
}

impl From<[u8; Token::BYTE_COUNT]> for Token {
    fn from(value: [u8; Token::BYTE_COUNT]) -> Self {
        let policy_id = PolicyId::from_raw_bytes(&value[0..PolicyId::BYTE_COUNT]).unwrap();
        let asset_name = AssetName::try_from(value[PolicyId::BYTE_COUNT..].to_vec()).unwrap();
        Self(policy_id, asset_name)
    }
}

impl From<Token> for Vec<u8> {
    fn from(value: Token) -> Self {
        <[u8; Token::BYTE_COUNT]>::from(value).into()
    }
}

impl TryFrom<&[u8]> for Token {
    type Error = ();
    fn try_from(value: &[u8]) -> Result<Self, Self::Error> {
        let policy_id = PolicyId::from_raw_bytes(&value[0..PolicyId::BYTE_COUNT]).map_err(|_| ())?;
        let asset_name = AssetName::try_from(value[PolicyId::BYTE_COUNT..].to_vec()).map_err(|_| ())?;
        Ok(Self(policy_id, asset_name))
    }
}

impl IntoPlutusData for Token {
    fn into_pd(self) -> PlutusData {
        PlutusData::ConstrPlutusData(ConstrPlutusData {
            alternative: 0,
            fields: vec![
                PlutusData::Bytes {
                    bytes: self.0.to_raw_bytes().to_vec(),
                    bytes_encoding: Default::default(),
                },
                PlutusData::Bytes {
                    bytes: self.1.as_bytes().to_vec(),
                    bytes_encoding: Default::default(),
                },
            ],
            encodings: Some(ConstrPlutusDataEncoding {
                len_encoding: Indefinite,
                tag_encoding: Some(cbor_event::Sz::Inline),
                alternative_encoding: None,
                fields_encoding: Indefinite,
                prefer_compact: true,
            }),
        })
    }
}

impl TryFromPData for Token {
    fn try_from_pd(data: PlutusData) -> Option<Self> {
        let mut cpd = data.into_constr_pd()?;
        let policy_bytes = cpd.take_field(0)?.into_bytes()?;
        let asset_name_bytes = cpd.take_field(1)?.into_bytes()?;
        if !policy_bytes.is_empty() {
            let policy_id = PolicyId::from_raw_bytes(&*policy_bytes).ok()?;
            let asset_name = AssetName::try_from(asset_name_bytes).ok()?;
            return Some(Token(policy_id, asset_name));
        }
        None
    }
}

impl Display for Token {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(format!("{}.{}", self.0.to_hex().to_string(), self.1).as_str())
    }
}

#[derive(Serialize, Deserialize)]
struct AssetClassFromHex(String);

impl TryFrom<AssetClassFromHex> for AssetClass {
    type Error = String;
    fn try_from(AssetClassFromHex(str): AssetClassFromHex) -> Result<Self, Self::Error> {
        hex::decode(str)
            .ok()
            .and_then(|bs| AssetClass::from_bytes(&*bs))
            .ok_or("".to_string())
    }
}

impl From<AssetClass> for AssetClassFromHex {
    fn from(value: AssetClass) -> Self {
        AssetClassFromHex(hex::encode(value.to_bytes()))
    }
}

#[derive(Debug, Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Serialize, Deserialize)]
#[serde(try_from = "AssetClassFromHex")]
#[serde(into = "AssetClassFromHex")]
pub enum AssetClass {
    Native,
    Token(Token),
}

impl AssetClass {
    pub fn is_native(&self) -> bool {
        matches!(self, AssetClass::Native)
    }

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
            AssetClass::Token(Token(policy, an)) => {
                let mut ma = MultiAsset::new();
                ma.set(policy, an.into(), amount);
                value.multiasset = ma;
            }
        }
        value
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let mut bf = vec![];
        match self {
            AssetClass::Native => {
                bf.push(0u8);
            }
            AssetClass::Token(Token(pol, an)) => {
                bf.append(&mut pol.to_raw_bytes().to_vec());
                bf.append(&mut an.as_bytes().to_vec());
            }
        }
        bf
    }

    pub fn from_bytes(bs: &[u8]) -> Option<Self> {
        let n = PolicyId::BYTE_COUNT;
        match bs {
            [0] => Some(AssetClass::Native),
            bs if bs.len() >= PolicyId::BYTE_COUNT => Some(AssetClass::Token(Token(
                PolicyId::from(<[u8; PolicyId::BYTE_COUNT]>::try_from(&bs[0..n]).ok()?),
                AssetName::from(cml_chain::assets::AssetName::new(bs[n..].to_vec()).ok()?),
            ))),
            _ => None,
        }
    }
}

impl From<Token> for AssetClass {
    fn from(value: Token) -> Self {
        Self::Token(value)
    }
}

impl Display for AssetClass {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            AssetClass::Native => f.write_str("Native"),
            AssetClass::Token(tkn) => Display::fmt(tkn, f),
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
            Some(AssetClass::Token(Token(policy_id, asset_name)))
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
#[derive(Derivative, Serialize, Deserialize)]
#[derivative(Debug(bound = ""), Copy(bound = ""), Clone(bound = ""), Eq(bound = ""))]
pub struct TaggedAmount<T>(u64, PhantomData<T>);

impl<T> Display for TaggedAmount<T> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        Display::fmt(&self.0, f)
    }
}

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

impl<T> CheckedSub for TaggedAmount<T> {
    fn checked_sub(&self, v: &Self) -> Option<Self> {
        self.0.checked_sub(v.0).map(TaggedAmount::new)
    }
}

impl<T> CheckedAdd for TaggedAmount<T> {
    fn checked_add(&self, v: &Self) -> Option<Self> {
        self.0.checked_add(v.0).map(|res| TaggedAmount::new(res))
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

#[derive(serde::Serialize, serde::Deserialize, Debug, Copy, Clone, From, Into, PartialEq, Eq)]
pub struct NetworkId(u8);
impl NetworkId {
    pub const PREPROD: Self = NetworkId(0);
    pub const MAINNET: Self = NetworkId(1);
}

/// Payment credential in bech32.
#[derive(serde::Deserialize, Debug, Clone, From, Into)]
pub struct PaymentCredential(String);

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub struct Ed25519PublicKey([u8; ED25519_PUB_KEY_LENGTH]);

impl From<[u8; ED25519_PUB_KEY_LENGTH]> for Ed25519PublicKey {
    fn from(value: [u8; ED25519_PUB_KEY_LENGTH]) -> Self {
        Self(value)
    }
}

impl TryFrom<Vec<u8>> for Ed25519PublicKey {
    type Error = ();

    fn try_from(value: Vec<u8>) -> Result<Self, Self::Error> {
        value.try_into().map(Self).map_err(|_| ())
    }
}

impl TryInto<PublicKey> for Ed25519PublicKey {
    type Error = DeserializeError;

    fn try_into(self) -> Result<PublicKey, Self::Error> {
        PublicKey::from_raw_bytes(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use crate::{AssetClass, AssetName, Token};
    use cml_chain::PolicyId;

    #[test]
    fn asset_name_is_isomorphic_to_cml() {
        let len = 14;
        let cml_an = cml_chain::assets::AssetName::new(vec![0u8; len]).unwrap();
        let spectrum_an = AssetName::from(cml_an.clone());
        let cml_an_reconstructed = cml_chain::assets::AssetName::from(spectrum_an);
        assert_eq!(cml_an, cml_an_reconstructed);
    }

    #[test]
    fn asset_class_encoding() {
        let ac = AssetClass::Token(Token(PolicyId::from([9u8; 28]), AssetName(2, [0u8; 32])));
        let encoded = ac.to_bytes();
        let decoded = AssetClass::from_bytes(&*encoded);
        assert_eq!(decoded, Some(ac));
    }
}
