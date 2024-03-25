use cml_chain::address::{Address, BaseAddress, EnterpriseAddress};
use cml_chain::certs::{Credential, StakeCredential};
use cml_chain::plutus::PlutusData;
use cml_crypto::{Ed25519KeyHash, RawBytesEncoding, ScriptHash};
use crate::NetworkId;
use crate::plutus_data::{ConstrPlutusDataExtension, PlutusDataExtension};
use crate::types::TryFromPData;

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
            0 => Some(PlutusCredential::PubKey(Ed25519KeyHash::from_raw_bytes(&*f0).ok()?)),
            1 => Some(PlutusCredential::Script(ScriptHash::from_raw_bytes(&*f0).ok()?)),
            _ => None,
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
pub struct PlutusAddress {
    pub payment_cred: PlutusCredential,
    pub stake_cred: Option<PlutusCredential>,
}

impl PlutusAddress {
    pub fn to_address(self, network_id: NetworkId) -> Address {
        let PlutusAddress {payment_cred, stake_cred} = self;
        match stake_cred {
            Some(stake_cred) => Address::Base(BaseAddress::new(network_id.into(), payment_cred.into(), stake_cred.into())),
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
