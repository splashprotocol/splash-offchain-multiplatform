use cml_chain::address::{Address, EnterpriseAddress};
use cml_chain::certs::StakeCredential;
use cml_crypto::{Bip32PrivateKey, Ed25519KeyHash, PrivateKey};
use derive_more::{From, Into};

use cardano_explorer::constants::get_network_id;
use spectrum_cardano_lib::PaymentCredential;

#[derive(serde::Deserialize, Debug, Clone, Into, From)]
pub struct OperatorRewardAddress(pub Address);

#[derive(serde::Deserialize, Debug, Copy, Clone, Into, From)]
pub struct OperatorCred(pub Ed25519KeyHash);

pub fn operator_creds(operator_sk_raw: &str, network_magic: u64) -> (PrivateKey, PaymentCredential, Address) {
    let network_id = get_network_id(network_magic);
    let operator_prv_bip32 = Bip32PrivateKey::from_bech32(operator_sk_raw).expect("wallet error");
    let operator_prv = operator_prv_bip32.to_raw_key();
    let operator_pk = operator_prv.to_public();
    let operator_pkh = operator_pk.hash();
    let addr = EnterpriseAddress::new(network_id, StakeCredential::new_pub_key(operator_pkh)).to_address();
    (
        operator_prv,
        operator_pkh.to_bech32("addr_vkh").unwrap().into(),
        addr,
    )
}
