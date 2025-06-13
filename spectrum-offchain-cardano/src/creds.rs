use cml_chain::address::{Address, BaseAddress, EnterpriseAddress, RewardAddress};
use cml_chain::certs::{Credential, StakeCredential};
use cml_crypto::{Bip32PrivateKey, Ed25519KeyHash, PrivateKey};
use derive_more::{From, Into};

use crate::funding::FundingAddresses;
use spectrum_cardano_lib::{NetworkId, PaymentCredential};

#[derive(serde::Deserialize, Debug, Clone, Into, From)]
pub struct OperatorRewardAddress(pub Address);

impl OperatorRewardAddress {
    pub fn address(self) -> Address {
        self.0
    }
}

#[derive(Debug, Clone, Into, From)]
pub struct CollateralAddress(pub Address);

impl CollateralAddress {
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

pub fn operator_creds(
    operator_sk_raw: &str,
    network_id: NetworkId,
) -> (OperatorCred, CollateralAddress, FundingAddresses<4>) {
    let operator_prv_bip32 = Bip32PrivateKey::from_bech32(operator_sk_raw).expect("wallet error");
    let operator_pk_main = operator_prv_bip32.to_public();

    let child_pkh_1 = operator_pk_main.derive(1).unwrap().to_raw_key().hash();
    let child_pkh_2 = operator_pk_main.derive(2).unwrap().to_raw_key().hash();
    let child_pkh_3 = operator_pk_main.derive(3).unwrap().to_raw_key().hash();
    let child_pkh_4 = operator_pk_main.derive(4).unwrap().to_raw_key().hash();

    let main_pkh = operator_pk_main.to_raw_key().hash();
    let main_paycred = Credential::new_pub_key(main_pkh);

    let network = network_id.into();

    let main_address = Address::Enterprise(EnterpriseAddress::new(network, main_paycred.clone()));

    let funding_address_1 = Address::Base(BaseAddress::new(
        network,
        main_paycred.clone(),
        Credential::new_pub_key(child_pkh_1),
    ));
    let funding_address_2 = Address::Base(BaseAddress::new(
        network,
        main_paycred.clone(),
        Credential::new_pub_key(child_pkh_2),
    ));
    let funding_address_3 = Address::Base(BaseAddress::new(
        network,
        main_paycred.clone(),
        Credential::new_pub_key(child_pkh_3),
    ));
    let funding_address_4 = Address::Base(BaseAddress::new(
        network,
        main_paycred,
        Credential::new_pub_key(child_pkh_4),
    ));

    let funding_addresses = [
        funding_address_1,
        funding_address_2,
        funding_address_3,
        funding_address_4,
    ];
    (main_pkh.into(), main_address.into(), funding_addresses.into())
}

pub fn operator_creds_base_address(
    operator_sk_raw: &str,
    network_id: NetworkId,
) -> (
    Address,
    RewardAddress,
    PaymentCredential,
    OperatorCred,
    PrivateKey,
) {
    let root_key = Bip32PrivateKey::from_bech32(operator_sk_raw).expect("wallet error");
    let account_key = root_key
        .derive(1852 + 0x80000000)
        .derive(1815 + 0x80000000)
        .derive(0x80000000);
    let payment_key = account_key.derive(0).derive(0).to_raw_key();
    let stake_key = account_key.derive(2).derive(0).to_raw_key();

    let payment_key_hash = payment_key.to_public().hash();
    let stake_key_hash = stake_key.to_public().hash();

    let addr = BaseAddress::new(
        network_id.into(),
        StakeCredential::new_pub_key(payment_key_hash),
        StakeCredential::new_pub_key(stake_key_hash),
    )
    .to_address();
    // Change to payment key
    let reward_addr = RewardAddress::new(network_id.into(), StakeCredential::new_pub_key(payment_key_hash));
    let encoded_addr = addr.to_bech32(None).unwrap();
    let payment_cred = payment_key_hash.to_bech32("addr_vkh").unwrap().into();
    println!("PAYMENT_CRED raw bytes: {:?}", payment_key_hash);
    println!(
        "ADDRESS raw bytes: {:?}",
        account_key.to_public().to_raw_key().hash()
    );
    println!("PAYMENT_CRED: {:?}", payment_cred);
    println!("ADDRESS: {:?}", encoded_addr);
    (
        addr,
        reward_addr,
        payment_cred,
        payment_key_hash.into(),
        payment_key,
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

        let funding_address_1 = Address::Base(BaseAddress::new(
            network,
            main_paycred.clone(),
            Credential::new_pub_key(child_pkh_1),
        ));
        let funding_address_2 = Address::Base(BaseAddress::new(
            network,
            main_paycred.clone(),
            Credential::new_pub_key(child_pkh_2),
        ));
        let funding_address_3 = Address::Base(BaseAddress::new(
            network,
            main_paycred.clone(),
            Credential::new_pub_key(child_pkh_3),
        ));
        let funding_address_4 = Address::Base(BaseAddress::new(
            network,
            main_paycred,
            Credential::new_pub_key(child_pkh_4),
        ));

        println!("operator_prv_bip32: {}", operator_prv_bip32.to_bech32());
        println!("operator pkh (main): {}", pkh_main);
        println!("stake pkh (1): {}", child_pkh_1);
        println!("stake pkh (2): {}", child_pkh_2);
        println!("stake pkh (3): {}", child_pkh_3);
        println!("stake pkh (4): {}", child_pkh_4);
        println!("address (main): {}", main_address.to_bech32(None).unwrap());
        println!(
            "funding address (1): {}",
            funding_address_1.to_bech32(None).unwrap()
        );
        println!(
            "funding address (2): {}",
            funding_address_2.to_bech32(None).unwrap()
        );
        println!(
            "funding address (3): {}",
            funding_address_3.to_bech32(None).unwrap()
        );
        println!(
            "funding address (4): {}",
            funding_address_4.to_bech32(None).unwrap()
        );

        assert_eq!(1, 1);
    }
}
