use cml_chain::address::{Address, EnterpriseAddress};
use cml_chain::certs::Credential;
use cml_crypto::{Bip32PrivateKey, Ed25519KeyHash, PrivateKey};
use derive_more::{From, Into};

use spectrum_cardano_lib::{NetworkId, PaymentCredential};

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

pub fn operator_creds(operator_sk_raw: &str, network_id: NetworkId) -> (PrivateKey, Address, OperatorCred) {
    let operator_prv_bip32 = Bip32PrivateKey::from_bech32(operator_sk_raw).expect("wallet error");
    let operator_prv = operator_prv_bip32.to_raw_key();
    let operator_pkh = operator_prv.to_public().hash();
    let main_address = Address::Enterprise(EnterpriseAddress::new(network_id.into(), Credential::new_pub_key(operator_pkh)));
    (
        operator_prv,
        main_address,
        operator_pkh.into(),
    )
}

#[cfg(test)]
mod tests {
    use cml_chain::address::{Address, BaseAddress, EnterpriseAddress};
    use cml_chain::certs::{Credential, StakeCredential};
    use cml_chain::genesis::network_info::NetworkInfo;
    use cml_crypto::Bip32PrivateKey;

    #[test]
    fn gen_operator_creds() {
        let network = NetworkInfo::mainnet().network_id();

        let operator_prv_bip32 = Bip32PrivateKey::generate_ed25519_bip32();
        let operator_pk_main = operator_prv_bip32.to_public();

        let child_pkh_1 = operator_pk_main.derive(1).unwrap().to_raw_key().hash();
        let child_pkh_2 = operator_pk_main.derive(2).unwrap().to_raw_key().hash();
        let child_pkh_3 = operator_pk_main.derive(3).unwrap().to_raw_key().hash();
        let child_pkh_4 = operator_pk_main.derive(4).unwrap().to_raw_key().hash();

        let pkh_main = operator_pk_main.to_raw_key().hash();
        let main_paycred = StakeCredential::new_pub_key(pkh_main);

        let main_address = Address::Enterprise(EnterpriseAddress::new(network, main_paycred.clone()));

        let funding_address_1 = Address::Base(BaseAddress::new(network, main_paycred.clone(), Credential::new_pub_key(child_pkh_1)));
        let funding_address_2 = Address::Base(BaseAddress::new(network, main_paycred.clone(), Credential::new_pub_key(child_pkh_2)));
        let funding_address_3 = Address::Base(BaseAddress::new(network, main_paycred.clone(), Credential::new_pub_key(child_pkh_3)));
        let funding_address_4 = Address::Base(BaseAddress::new(network, main_paycred, Credential::new_pub_key(child_pkh_4)));

        println!("operator_prv_bip32: {}", operator_prv_bip32.to_bech32());
        println!("operator pkh (main): {}", pkh_main);
        println!("stake pkh (1): {}", child_pkh_1);
        println!("stake pkh (2): {}", child_pkh_2);
        println!("stake pkh (3): {}", child_pkh_3);
        println!("stake pkh (4): {}", child_pkh_4);
        println!("address (main): {}", main_address.to_bech32(None).unwrap());
        println!("funding address (1): {}", funding_address_1.to_bech32(None).unwrap());
        println!("funding address (2): {}", funding_address_2.to_bech32(None).unwrap());
        println!("funding address (3): {}", funding_address_3.to_bech32(None).unwrap());
        println!("funding address (4): {}", funding_address_4.to_bech32(None).unwrap());

        assert_eq!(1, 1);
    }
}
