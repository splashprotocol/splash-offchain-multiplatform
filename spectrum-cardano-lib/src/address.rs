use crate::plutus_data::{ConstrPlutusDataExtension, IntoPlutusData, PlutusDataExtension};
use crate::types::TryFromPData;
use crate::NetworkId;
use cml_chain::address::{Address, BaseAddress, EnterpriseAddress};
use cml_chain::certs::{Credential, StakeCredential};
use cml_chain::plutus::utils::ConstrPlutusDataEncoding;
use cml_chain::plutus::{ConstrPlutusData, PlutusData};
use cml_core::serialization::LenEncoding::{Canonical, Indefinite};
use cml_crypto::{Ed25519KeyHash, RawBytesEncoding, ScriptHash};

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub enum PlutusCredential {
    PubKey(Ed25519KeyHash),
    Script(ScriptHash),
}

impl TryFromPData for PlutusCredential {
    fn try_from_pd(data: PlutusData) -> Option<Self> {
        let mut cpd = data.into_constr_pd()?;
        let f0 = cpd.take_field(0)?.into_bytes()?;
        match cpd.alternative {
            0 => Some(PlutusCredential::PubKey(
                Ed25519KeyHash::from_raw_bytes(&*f0).ok()?,
            )),
            1 => Some(PlutusCredential::Script(ScriptHash::from_raw_bytes(&*f0).ok()?)),
            _ => None,
        }
    }
}

impl IntoPlutusData for PlutusCredential {
    fn into_pd(self) -> PlutusData {
        match self {
            PlutusCredential::PubKey(pub_key_hash) => PlutusData::new_constr_plutus_data(ConstrPlutusData {
                alternative: 0,
                fields: vec![PlutusData::new_bytes(pub_key_hash.to_raw_bytes().to_vec())],
                encodings: Some(ConstrPlutusDataEncoding {
                    len_encoding: Canonical,
                    tag_encoding: Some(cbor_event::Sz::One),
                    alternative_encoding: None,
                    fields_encoding: Indefinite,
                    prefer_compact: true,
                }),
            }),
            PlutusCredential::Script(script_hash) => PlutusData::new_constr_plutus_data(ConstrPlutusData {
                alternative: 1,
                fields: vec![PlutusData::new_bytes(script_hash.to_raw_bytes().to_vec())],
                encodings: Some(ConstrPlutusDataEncoding {
                    len_encoding: Canonical,
                    tag_encoding: Some(cbor_event::Sz::One),
                    alternative_encoding: None,
                    fields_encoding: Indefinite,
                    prefer_compact: true,
                }),
            }),
        }
    }
}

impl From<PlutusCredential> for Credential {
    fn from(value: PlutusCredential) -> Self {
        match value {
            PlutusCredential::PubKey(hash) => Credential::new_pub_key(hash),
            PlutusCredential::Script(hash) => Credential::new_script(hash),
        }
    }
}

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub struct InlineCredential(PlutusCredential);

impl InlineCredential {
    pub fn script_hash(self) -> Option<ScriptHash> {
        match self {
            InlineCredential(PlutusCredential::Script(script_hash)) => Some(script_hash),
            _ => None,
        }
    }
}

impl From<ScriptHash> for InlineCredential {
    fn from(value: ScriptHash) -> Self {
        Self(PlutusCredential::Script(value))
    }
}

impl TryFromPData for InlineCredential {
    fn try_from_pd(data: PlutusData) -> Option<Self> {
        let mut cpd = data.into_constr_pd()?;
        match cpd.alternative {
            0 => Some(InlineCredential(PlutusCredential::try_from_pd(
                cpd.take_field(0)?,
            )?)),
            _ => None,
        }
    }
}

impl IntoPlutusData for InlineCredential {
    fn into_pd(self) -> PlutusData {
        PlutusData::new_constr_plutus_data(ConstrPlutusData {
            alternative: 0,
            fields: vec![self.0.into_pd()],
            encodings: Some(ConstrPlutusDataEncoding {
                len_encoding: Canonical,
                tag_encoding: Some(cbor_event::Sz::One),
                alternative_encoding: None,
                fields_encoding: Indefinite,
                prefer_compact: true,
            }),
        })
    }
}

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub struct PlutusAddress {
    pub payment_cred: PlutusCredential,
    pub stake_cred: Option<InlineCredential>,
}

impl PlutusAddress {
    pub fn to_address(self, network_id: NetworkId) -> Address {
        let PlutusAddress {
            payment_cred,
            stake_cred,
        } = self;
        match stake_cred {
            Some(InlineCredential(stake_cred)) => Address::Base(BaseAddress::new(
                network_id.into(),
                payment_cred.into(),
                stake_cred.into(),
            )),
            None => Address::Enterprise(EnterpriseAddress::new(network_id.into(), payment_cred.into())),
        }
    }
}

impl TryFromPData for PlutusAddress {
    fn try_from_pd(data: PlutusData) -> Option<Self> {
        let mut cpd = data.into_constr_pd()?;
        Some(PlutusAddress {
            payment_cred: TryFromPData::try_from_pd(cpd.take_field(0)?)?,
            stake_cred: TryFromPData::try_from_pd(cpd.take_field(1)?)?,
        })
    }
}

impl IntoPlutusData for PlutusAddress {
    fn into_pd(self) -> PlutusData {
        PlutusData::new_constr_plutus_data(ConstrPlutusData {
            alternative: 0,
            fields: vec![self.payment_cred.into_pd(), self.stake_cred.into_pd()],
            encodings: Some(ConstrPlutusDataEncoding {
                len_encoding: Canonical,
                tag_encoding: Some(cbor_event::Sz::One),
                alternative_encoding: None,
                fields_encoding: Indefinite,
                prefer_compact: true,
            }),
        })
    }
}

pub trait AddressExtension {
    fn script_hash(&self) -> Option<ScriptHash>;
    fn update_staking_cred(&mut self, cred: Credential);

    fn update_payment_cred(&mut self, cred: Credential);
}

impl AddressExtension for Address {
    fn script_hash(&self) -> Option<ScriptHash> {
        match self.payment_cred()? {
            StakeCredential::PubKey { .. } => None,
            StakeCredential::Script { hash, .. } => Some(*hash),
        }
    }
    fn update_staking_cred(&mut self, cred: Credential) {
        match self {
            Self::Base(ref mut a) => {
                a.stake = cred;
            }
            _ => {}
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

#[cfg(test)]
mod test {
    use crate::address::PlutusAddress;
    use crate::types::TryFromPData;
    use cml_chain::plutus::PlutusData;
    use cml_core::serialization::Deserialize;

    const RAW_ADDR: &str = "D8799FD8799F581C4BE4FA25F029D14C0D723AF4A1E6FA7133FC3A610F880336AD685CBAFFD8799FD8799FD8799F581C5BDA73043D43AD8DF5CE75639CF48E1F2B4545403BE92F0113E37537FFFFFFFF";

    #[test]
    fn decode_address() {
        let addr: PlutusAddress =
            TryFromPData::try_from_pd(PlutusData::from_cbor_bytes(&*hex::decode(RAW_ADDR).unwrap()).unwrap())
                .unwrap();
        dbg!(addr);
    }
}
