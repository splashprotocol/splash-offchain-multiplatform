use std::collections::HashMap;
use std::io::Error;
use std::path::Path;

use cml_chain::builders::tx_builder::TransactionUnspentOutput;
use cml_chain::transaction::{TransactionInput, TransactionOutput};
use cml_core::serialization::Deserialize;
use cml_crypto::TransactionHash;
use maestro::utils::Parameters;
use tokio::fs;

use crate::constants::{MAINNET_PREFIX, PREPROD_PREFIX};
use spectrum_cardano_lib::{NetworkId, OutputRef, PaymentCredential};

use crate::Network::{Mainnet, Preprod};

pub mod client;

pub mod constants;
pub mod data;

#[derive(serde::Deserialize)]
pub enum Network {
    Preprod,
    Mainnet,
}

impl From<NetworkId> for Network {
    fn from(value: NetworkId) -> Self {
        match <u8>::from(value) {
            0 => Preprod,
            _ => Mainnet,
        }
    }
}

impl From<Network> for String {
    fn from(value: Network) -> Self {
        match value {
            Preprod => PREPROD_PREFIX.to_string(),
            Mainnet => MAINNET_PREFIX.to_string(),
        }
    }
}

pub trait CardanoNetwork {
    async fn utxo_by_ref(&self, oref: OutputRef) -> Option<TransactionUnspentOutput>;
    async fn utxos_by_pay_cred(
        &self,
        payment_credential: PaymentCredential,
        offset: u32,
        limit: u16,
    ) -> Vec<TransactionUnspentOutput>;
}

pub struct Maestro(maestro::Maestro);

impl Maestro {
    pub async fn new<P: AsRef<Path>>(path: P, network: Network) -> Result<Self, Error> {
        let token = fs::read_to_string(path).await?.replace("\n", "");
        Ok(Self(maestro::Maestro::new(token, network.into())))
    }
}

impl CardanoNetwork for Maestro {
    async fn utxo_by_ref(&self, oref: OutputRef) -> Option<TransactionUnspentOutput> {
        let params = Some(HashMap::from([(
            "with_cbor".to_lowercase(),
            "true".to_lowercase(),
        )]));
        self.0
            .transaction_output_from_reference(oref.tx_hash().to_hex().as_str(), oref.index() as i32, params)
            .await
            .and_then(|tx_out| {
                let tx_out = TransactionOutput::from_cbor_bytes(&*hex::decode(tx_out.data.tx_out_cbor)?)?;
                Ok(TransactionUnspentOutput::new(oref.into(), tx_out))
            })
            .ok()
    }

    async fn utxos_by_pay_cred(
        &self,
        payment_credential: PaymentCredential,
        offset: u32,
        limit: u16,
    ) -> Vec<TransactionUnspentOutput> {
        let mut params = Parameters::new();
        params.with_cbor();
        params.from(offset as i64);
        params.count(limit as i32);
        self.0
            .utxos_by_payment_credential(String::from(payment_credential).as_str(), Some(params))
            .await
            .and_then(|resp| {
                let mut utxos = vec![];
                for utxo in resp.data {
                    let tx_in = TransactionInput::new(
                        TransactionHash::from_hex(utxo.tx_hash.as_str()).unwrap(),
                        utxo.index as u64,
                    );
                    let tx_out = TransactionOutput::from_cbor_bytes(&*hex::decode(utxo.tx_out_cbor)?)?;
                    utxos.push(TransactionUnspentOutput::new(tx_in, tx_out));
                }
                Ok(utxos)
            })
            .ok()
            .unwrap_or(vec![])
    }
}
