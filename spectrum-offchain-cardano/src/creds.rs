use cml_chain::address::{Address, EnterpriseAddress};
use cml_chain::certs::StakeCredential;
use cml_chain::genesis::network_info::NetworkInfo;
use cml_crypto::{Bip32PrivateKey, Ed25519KeyHash, PrivateKey};

pub fn operator_creds(
    operator_sk_raw: &str,
    network_info: NetworkInfo,
) -> (PrivateKey, Ed25519KeyHash, Address) {
    let operator_prv_bip32 = Bip32PrivateKey::from_bech32(operator_sk_raw).expect("wallet error");
    let operator_prv = operator_prv_bip32.to_raw_key();
    let operator_pkh = operator_prv.to_public().hash();
    let addr = EnterpriseAddress::new(
        network_info.network_id(),
        StakeCredential::new_pub_key(operator_pkh),
    )
    .to_address();
    (operator_prv, operator_pkh, addr)
}
