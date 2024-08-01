use cml_chain::address::{Address, EnterpriseAddress};
use cml_chain::certs::{Credential, StakeCredential};
use cml_chain::genesis::network_info::NetworkInfo;
use cml_crypto::{Bip32PrivateKey, Ed25519KeyHash, PrivateKey};
use derive_more::{From, Into};

use cardano_explorer::constants::get_network_id;
use spectrum_cardano_lib::PaymentCredential;

#[derive(serde::Deserialize, Debug, Clone, Into, From)]
pub struct OperatorRewardAddress(pub Address);

impl OperatorRewardAddress {
    pub fn address(self) -> Address {
        self.0
    }
}

#[derive(serde::Deserialize, Debug, Copy, Clone, Into, From)]
pub struct OperatorCred(pub Ed25519KeyHash);

impl From<OperatorCred> for Credential {
    fn from(value: OperatorCred) -> Self {
        Credential::PubKey {
            hash: value.0,
            len_encoding: Default::default(),
            tag_encoding: None,
            hash_encoding: Default::default(),
        }
    }
}

pub fn operator_creds(operator_sk_raw: &str) -> (PrivateKey, PaymentCredential, OperatorCred) {
    let operator_prv_bip32 = Bip32PrivateKey::from_bech32(operator_sk_raw).expect("wallet error");
    let operator_prv = operator_prv_bip32.to_raw_key();
    let operator_pk = operator_prv.to_public();
    let operator_pkh = operator_pk.hash();
    (
        operator_prv,
        operator_pkh.to_bech32("addr_vkh").unwrap().into(),
        operator_pkh.into(),
    )
}
