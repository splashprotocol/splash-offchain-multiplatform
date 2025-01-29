use async_trait::async_trait;
use blockfrost::{BlockFrostSettings, BlockfrostAPI, BlockfrostResult, Order, Pagination, RetrySettings};
use blockfrost_openapi::models::{
    AddressUtxoContentInner, TxContentOutputAmountInner, TxContentUtxoOutputsInner,
};
use cml_chain::address::Address;
use cml_chain::builders::tx_builder::TransactionUnspentOutput;
use cml_chain::plutus::{PlutusData, PlutusV2Script};
use cml_chain::transaction::{DatumOption, ScriptRef, TransactionInput, TransactionOutput};
use cml_chain::Script::PlutusV2;
use cml_chain::{Script, Value};
use cml_core::serialization::Deserialize;
use cml_crypto::chain_core::property::TransactionId;
use cml_crypto::{DatumHash, TransactionHash};
use futures::future::join_all;
use futures::{FutureExt, TryFutureExt};
use isahc::{AsyncReadResponseExt, Request};
use maestro_rust_sdk::client::maestro;
use maestro_rust_sdk::models::addresses::UtxosAtAddress;
use maestro_rust_sdk::models::transactions::RedeemerEvaluation;
use maestro_rust_sdk::utils::Parameters;
use pallas_primitives::alonzo::Value::Multiasset;
use std::collections::HashMap;
use std::io::Error;
use std::num::ParseIntError;
use std::path::Path;
use std::string::ToString;
use std::time::Duration;
use tokio::fs;

use crate::constants::{MAINNET_PREFIX, PREPROD_PREFIX};
use spectrum_cardano_lib::value::ValueExtension;
use spectrum_cardano_lib::AssetClass::{Native, Token};
use spectrum_cardano_lib::Token as RawToken;
use spectrum_cardano_lib::{NetworkId, OutputRef, PaymentCredential};

use crate::Network::{Mainnet, Preprod};

pub mod client;

pub mod constants;
pub mod data;
pub mod retry;

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

#[async_trait]
pub trait CardanoNetwork: Send + Sync {
    async fn utxo_by_ref(&self, oref: OutputRef) -> Option<TransactionUnspentOutput>;
    async fn utxos_by_pay_cred(
        &self,
        payment_credential: PaymentCredential,
        offset: u32,
        limit: u16,
    ) -> Vec<TransactionUnspentOutput>;
    async fn utxos_by_address(
        &self,
        address: Address,
        offset: u32,
        limit: u16,
    ) -> Vec<TransactionUnspentOutput>;
}

pub trait ExtendedCardanoNetwork: CardanoNetwork {
    async fn slot_indexed_utxos_by_address(
        &self,
        address: Address,
        offset: u32,
        limit: u16,
    ) -> Vec<(TransactionUnspentOutput, u64)>;
    async fn submit_tx(&self, cbor: &[u8]) -> Result<(), Box<dyn std::error::Error>>;
    async fn chain_tip_slot_number(&self) -> Result<u64, Box<dyn std::error::Error>>;
    async fn wait_for_transaction_confirmation(
        &self,
        tx_id: TransactionHash,
    ) -> Result<(), Box<dyn std::error::Error>>;
    async fn evaluate_tx(&self, cbor: &str) -> Result<Vec<RedeemerEvaluation>, Box<dyn std::error::Error>>;
}

const LOVELACE: &str = "lovelace";

pub struct Blockfrost(BlockfrostAPI);

impl Blockfrost {
    pub async fn new<P: AsRef<Path>>(path: P) -> Result<Self, Error> {
        let project_id = fs::read_to_string(path).await?.replace("\n", "");
        let settings = BlockFrostSettings::new();
        let blockfrost_client = BlockfrostAPI::new(project_id.as_str(), settings);

        Ok(Blockfrost(blockfrost_client))
    }

    async fn parse_blockfrost_output(
        &self,
        tx_hash: String,
        output_idx: u64,
        address: String,
        output_amount: Vec<TxContentOutputAmountInner>,
        inline_datum: Option<String>,
        datum_hash: Option<String>,
        ref_script_hash_opt: Option<String>,
    ) -> Option<TransactionUnspentOutput> {
        let mut script = None;
        if let Some(ref_script_hash) = ref_script_hash_opt {
            script = self
                .0
                .scripts_hash_cbor(ref_script_hash.as_str())
                .await
                .ok()
                .and_then(|opt_value| opt_value.cbor)
                .and_then(|script| PlutusV2Script::from_cbor_bytes(script.as_ref()).ok())
                .map(Script::new_plutus_v2)
        }

        let mut value = Value::zero();
        output_amount.into_iter().for_each(|token_info| {
            token_info
                .quantity
                .parse::<u64>()
                .into_iter()
                .for_each(|token_qty| match token_info.clone().unit.as_str() {
                    LOVELACE => value.add_unsafe(Native, token_qty),
                    csWithTn => RawToken::try_from_raw_string(csWithTn)
                        .into_iter()
                        .for_each(|token| value.add_unsafe(Token(token), token_qty)),
                })
        });

        let datum: Option<DatumOption> = inline_datum
            .clone()
            .and_then(|datum| {
                PlutusData::from_cbor_bytes(datum.as_ref())
                    .ok()
                    .map(DatumOption::new_datum)
            })
            .or(datum_hash.and_then(|datum_hash| {
                DatumHash::from_hex(datum_hash.as_str())
                    .ok()
                    .map(DatumOption::new_hash)
            }));

        return Some(TransactionUnspentOutput {
            input: TransactionInput::new(TransactionHash::from_hex(tx_hash.as_str()).ok()?, output_idx),
            output: TransactionOutput::new(
                Address::from_bech32(address.as_str()).ok()?,
                value,
                datum,
                script,
            ),
        });
    }

    async fn blockfrost_address_utxo_to_tx_unspent_output(
        &self,
        utxo: AddressUtxoContentInner,
    ) -> Option<TransactionUnspentOutput> {
        self.parse_blockfrost_output(
            utxo.tx_hash,
            utxo.output_index as u64,
            utxo.address,
            utxo.amount,
            utxo.inline_datum,
            utxo.data_hash,
            utxo.reference_script_hash,
        )
        .await
    }

    async fn blockfrost_tx_utxo_to_tx_unspent_output(
        &self,
        output_ref: OutputRef,
        utxo: TxContentUtxoOutputsInner,
    ) -> Option<TransactionUnspentOutput> {
        self.parse_blockfrost_output(
            output_ref.tx_hash().to_hex(),
            output_ref.index(),
            utxo.address,
            utxo.amount,
            utxo.inline_datum,
            utxo.data_hash,
            utxo.reference_script_hash,
        )
        .await
    }
}

#[async_trait]
impl CardanoNetwork for Blockfrost {
    async fn utxo_by_ref(&self, oref: OutputRef) -> Option<TransactionUnspentOutput> {
        let transaction_outputs = self
            .0
            .transactions_utxos(oref.tx_hash().to_hex().as_str())
            .await
            .ok()?;

        if let Some(output) = transaction_outputs
            .outputs
            .into_iter()
            .find(|output| output.output_index as u64 == oref.index())
        {
            return self.blockfrost_tx_utxo_to_tx_unspent_output(oref, output).await;
        };

        return None;
    }

    async fn utxos_by_pay_cred(
        &self,
        payment_credential: PaymentCredential,
        offset: u32,
        limit: u16,
    ) -> Vec<TransactionUnspentOutput> {
        let outputs = self
            .0
            .addresses_utxos(
                String::from(payment_credential.clone()).as_str(),
                Pagination {
                    fetch_all: false,
                    count: limit as usize,
                    // blockfrost pagination start from page 1
                    page: ((offset / limit as u32) + 1) as usize,
                    order: Order::Asc,
                },
            )
            .await
            .unwrap_or(vec![]);

        let parsed_outputs: Vec<_> = outputs
            .into_iter()
            .map(|output| async move { self.blockfrost_address_utxo_to_tx_unspent_output(output).await })
            .collect();

        join_all(parsed_outputs).await.into_iter().flatten().collect()
    }

    async fn utxos_by_address(
        &self,
        address: Address,
        offset: u32,
        limit: u16,
    ) -> Vec<TransactionUnspentOutput> {
        let outputs = self
            .0
            .addresses_utxos(
                String::from(address.to_bech32(None).unwrap().as_str()).as_str(),
                Pagination {
                    fetch_all: false,
                    count: limit as usize,
                    // blockfrost pagination start from page 1
                    page: ((offset / limit as u32) + 1) as usize,
                    order: Order::Asc,
                },
            )
            .await
            .unwrap_or(vec![]);

        let parsed_outputs: Vec<_> = outputs
            .into_iter()
            .map(|output| async move { self.blockfrost_address_utxo_to_tx_unspent_output(output).await })
            .collect();

        join_all(parsed_outputs).await.into_iter().flatten().collect()
    }
}

pub struct Maestro(maestro::Maestro);

impl Maestro {
    pub async fn new<P: AsRef<Path>>(path: P, network: Network) -> Result<Self, Error> {
        let token = fs::read_to_string(path).await?.replace("\n", "");
        Ok(Self(maestro::Maestro::new(token, network.into())))
    }
}

#[async_trait]
impl CardanoNetwork for Maestro {
    async fn utxo_by_ref(&self, oref: OutputRef) -> Option<TransactionUnspentOutput> {
        let params = Some(HashMap::from([(
            "with_cbor".to_lowercase(),
            "true".to_lowercase(),
        )]));
        retry!(self
            .0
            .transaction_output_from_reference(
                oref.tx_hash().to_hex().as_str(),
                oref.index() as i32,
                params.clone()
            )
            .await
            .ok())
        .and_then(|tx_out| {
            let tx_out =
                TransactionOutput::from_cbor_bytes(&hex::decode(tx_out.data.tx_out_cbor?).ok()?).ok()?;
            Some(TransactionUnspentOutput::new(oref.into(), tx_out))
        })
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
        retry!(self
            .0
            .utxos_by_payment_credential(
                String::from(payment_credential.clone()).as_str(),
                Some(params.clone())
            )
            .await
            .ok())
        .and_then(|utxos| read_maestro_utxos(utxos).ok())
        .unwrap_or(vec![])
    }

    async fn utxos_by_address(
        &self,
        address: Address,
        offset: u32,
        limit: u16,
    ) -> Vec<TransactionUnspentOutput> {
        let mut params = Parameters::new();
        params.with_cbor();
        params.from(offset as i64);
        params.count(limit as i32);
        retry!(self
            .0
            .utxos_at_address(address.to_bech32(None).unwrap().as_str(), Some(params.clone()))
            .await
            .ok())
        .and_then(|utxos| read_maestro_utxos(utxos).ok())
        .unwrap_or(vec![])
    }
}

impl ExtendedCardanoNetwork for Maestro {
    async fn slot_indexed_utxos_by_address(
        &self,
        address: Address,
        offset: u32,
        limit: u16,
    ) -> Vec<(TransactionUnspentOutput, u64)> {
        let utxos = self.utxos_by_address(address, offset, limit).await;
        let mut res = vec![];

        for utxo in utxos {
            let tx_details = self
                .0
                .transaction_details(&utxo.input.transaction_id.to_hex())
                .await
                .unwrap();
            res.push((utxo, tx_details.data.block_absolute_slot as u64));
        }
        res
    }

    async fn submit_tx(&self, cbor_bytes: &[u8]) -> Result<(), Box<dyn std::error::Error>> {
        let result = self.0.tx_manager_submit(cbor_bytes.to_vec()).await?;
        println!("TX submit result: {}", result);
        Ok(())
    }

    async fn chain_tip_slot_number(&self) -> Result<u64, Box<dyn std::error::Error>> {
        let r = self.0.chain_tip().await?;
        Ok(r.last_updated.block_slot as u64)
    }

    async fn evaluate_tx(&self, cbor: &str) -> Result<Vec<RedeemerEvaluation>, Box<dyn std::error::Error>> {
        self.0.evaluate_tx(cbor, vec![]).await
    }

    async fn wait_for_transaction_confirmation(
        &self,
        tx_id: TransactionHash,
    ) -> Result<(), Box<dyn std::error::Error>> {
        while self
            .0
            .transaction_cbor(&tx_id.to_hex())
            .await
            .map(|_| ())
            .is_err()
        {
            tokio::time::sleep(std::time::Duration::from_secs(30)).await;
        }
        Ok(())
    }
}

fn read_maestro_utxos(
    resp: UtxosAtAddress,
) -> Result<Vec<TransactionUnspentOutput>, Box<dyn std::error::Error>> {
    let mut utxos = vec![];
    for utxo in resp.data {
        let tx_in = TransactionInput::new(
            TransactionHash::from_hex(utxo.tx_hash.as_str()).unwrap(),
            utxo.index as u64,
        );
        let tx_out = TransactionOutput::from_cbor_bytes(&*hex::decode(utxo.tx_out_cbor.unwrap())?)?;
        utxos.push(TransactionUnspentOutput::new(tx_in, tx_out));
    }
    Ok(utxos)
}
