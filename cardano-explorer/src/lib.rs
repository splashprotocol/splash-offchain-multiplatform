use crate::config::ExplorerConfig;
use crate::constants::{MAINNET_PREFIX, PREPROD_PREFIX, PREVIEW_PREFIX};
use crate::Network::{Mainnet, Preprod};
use async_trait::async_trait;
use blockfrost::{BlockFrostSettings, BlockfrostAPI, Order, Pagination};
use blockfrost_openapi::models::{
    AddressUtxoContentInner, TxContentMetadataCborInner, TxContentOutputAmountInner,
};
use cml_chain::address::Address;
use cml_chain::auxdata::{Metadata, TransactionMetadatum};
use cml_chain::builders::tx_builder::TransactionUnspentOutput;
use cml_chain::plutus::PlutusData;
use cml_chain::transaction::{DatumOption, Transaction, TransactionBody, TransactionOutput};
use cml_chain::Value;
use cml_core::serialization::Deserialize;
use cml_core::Int;
use cml_crypto::{DatumHash, TransactionHash};
use futures::future::join_all;
use log::{trace, warn};
use spectrum_cardano_lib::value::ValueExtension;
use spectrum_cardano_lib::AssetClass::{Native, Token};
use spectrum_cardano_lib::Token as RawToken;
use spectrum_cardano_lib::{NetworkId, OutputRef, PaymentCredential};
use std::future::Future;
use std::io::{Error, ErrorKind};
use std::path::Path;
use std::string::ToString;
use tokio::fs;

pub mod client;

pub mod config;
pub mod constants;
pub mod data;
pub mod retry;

#[cfg(test)]
mod tests;

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
pub trait CardanoNetwork: Sized {
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

#[async_trait]
impl<T: CardanoNetwork + Sync> CardanoNetwork for Box<T> {
    async fn utxo_by_ref(&self, oref: OutputRef) -> Option<TransactionUnspentOutput> {
        self.as_ref().utxo_by_ref(oref).await
    }

    async fn utxos_by_pay_cred(
        &self,
        payment_credential: PaymentCredential,
        offset: u32,
        limit: u16,
    ) -> Vec<TransactionUnspentOutput> {
        self.as_ref()
            .utxos_by_pay_cred(payment_credential, offset, limit)
            .await
    }

    async fn utxos_by_address(
        &self,
        address: Address,
        offset: u32,
        limit: u16,
    ) -> Vec<TransactionUnspentOutput> {
        self.as_ref().utxos_by_address(address, offset, limit).await
    }
}

pub trait ExtendedCardanoNetwork: CardanoNetwork {
    async fn slot_indexed_utxos_by_address(&self, address: Address, offset: u32, limit: u16)
        -> Vec<UTxOInfo>;
    async fn submit_tx(&self, cbor: &[u8]) -> Result<(), Box<dyn std::error::Error>>;
    async fn chain_tip_slot_number(&self) -> Result<u64, Box<dyn std::error::Error>>;
    async fn wait_for_transaction_confirmation(
        &self,
        tx_id: TransactionHash,
    ) -> Result<(), Box<dyn std::error::Error>>;
}

const LOVELACE: &str = "lovelace";

pub struct Blockfrost(BlockfrostAPI);

impl Blockfrost {
    pub async fn new<P: AsRef<Path>>(path: P, network_id: NetworkId) -> Result<Self, Error> {
        let project_id = project_id_from_key_file(fs::read_to_string(path).await?.as_str(), network_id)?;
        let settings = BlockFrostSettings::new();
        let blockfrost_client = BlockfrostAPI::new(project_id.as_str(), settings);

        Ok(Blockfrost(blockfrost_client))
    }

    /// Resolves an output from the CBOR of the transaction that produced it. Unlike the JSON
    /// representation this keeps the inline datum and the reference script of any Plutus version.
    async fn output_from_tx_cbor(&self, oref: OutputRef) -> Option<TransactionOutput> {
        // Only the request is retried. Decoding and the index lookup are deterministic, and an
        // output that legitimately isn't there must fail immediately instead of re-downloading the
        // whole transaction 25 times and turning a plain miss into a rate-limit storm.
        let tx_cbor = retry!(self
            .0
            .transactions_cbor(oref.tx_hash().to_hex().as_str())
            .await
            .ok())?
        .cbor;
        let body = decode_tx_body(tx_cbor.as_str())?;
        // The extra request only ever fires on the collateral-return miss path, never on a normal
        // lookup.
        select_output(body, oref.index() as usize, || async {
            let details = retry!(self
                .0
                .transaction_by_hash(oref.tx_hash().to_hex().as_str())
                .await
                .ok())?;
            Some(details.valid_contract)
        })
        .await
    }

    /// Transaction metadata, decoded from its CBOR. The JSON representation cannot be used: the
    /// metadatum type it deserializes into has no number variant, so a single numeric metadatum
    /// fails the whole response.
    async fn tx_metadata(&self, tx_hash: &str) -> Option<Metadata> {
        // A TX without metadata is not an error, it just comes back with an empty listing.
        let entries = retry!(self.0.transactions_metadata_cbor(tx_hash).await.ok())?;
        metadata_from_cbor_entries(tx_hash, entries)
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
        let oref = OutputRef::new(TransactionHash::from_hex(tx_hash.as_str()).ok()?, output_idx);

        // The listing doesn't say which Plutus version a reference script has, so outputs
        // carrying one are resolved from the tx CBOR, which does.
        if ref_script_hash_opt.is_some() {
            return self
                .output_from_tx_cbor(oref)
                .await
                .map(|output| TransactionUnspentOutput::new(oref.into(), output));
        }

        output_from_listing(address.as_str(), output_amount, inline_datum, datum_hash)
            .map(|output| TransactionUnspentOutput::new(oref.into(), output))
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
}

#[async_trait]
impl CardanoNetwork for Blockfrost {
    async fn utxo_by_ref(&self, oref: OutputRef) -> Option<TransactionUnspentOutput> {
        self.output_from_tx_cbor(oref)
            .await
            .map(|output| TransactionUnspentOutput::new(oref.into(), output))
    }

    async fn utxos_by_pay_cred(
        &self,
        payment_credential: PaymentCredential,
        offset: u32,
        limit: u16,
    ) -> Vec<TransactionUnspentOutput> {
        if let Some(page_size) = blockfrost_page(offset, limit) {
            let outputs = retry!(self
                .0
                .addresses_utxos(
                    String::from(payment_credential.clone()).as_str(),
                    Pagination {
                        fetch_all: false,
                        count: limit as usize,
                        page: page_size as usize,
                        order: Order::Asc,
                    },
                )
                .await
                .ok())
            .unwrap_or(vec![]);

            let parsed_outputs: Vec<_> = outputs
                .into_iter()
                .map(|output| async move { self.blockfrost_address_utxo_to_tx_unspent_output(output).await })
                .collect();

            return join_all(parsed_outputs).await.into_iter().flatten().collect();
        }

        vec![]
    }

    async fn utxos_by_address(
        &self,
        address: Address,
        offset: u32,
        limit: u16,
    ) -> Vec<TransactionUnspentOutput> {
        if let Some(page_size) = blockfrost_page(offset, limit) {
            let outputs = retry!(self
                .0
                .addresses_utxos(
                    String::from(address.to_bech32(None).unwrap().as_str()).as_str(),
                    Pagination {
                        fetch_all: false,
                        count: limit as usize,
                        page: page_size as usize,
                        order: Order::Asc,
                    },
                )
                .await
                .ok())
            .unwrap_or(vec![]);

            let parsed_outputs: Vec<_> = outputs
                .into_iter()
                .map(|output| async move { self.blockfrost_address_utxo_to_tx_unspent_output(output).await })
                .collect();

            return join_all(parsed_outputs).await.into_iter().flatten().collect();
        };

        vec![]
    }
}

impl ExtendedCardanoNetwork for Blockfrost {
    async fn slot_indexed_utxos_by_address(
        &self,
        address: Address,
        offset: u32,
        limit: u16,
    ) -> Vec<UTxOInfo> {
        let utxos = self.utxos_by_address(address, offset, limit).await;
        let mut res = vec![];

        for utxo in utxos {
            let tx_hash = utxo.input.transaction_id.to_hex();
            // This runs once per UTxO of a page, so a single rate-limited request must not be
            // allowed to take the whole pull down.
            let Some(tx_details) = retry!(self.0.transaction_by_hash(tx_hash.as_str()).await.ok()) else {
                warn!("Skipping UTxO of TX {}: details could not be fetched", tx_hash);
                continue;
            };
            let info = UTxOInfo {
                utxo,
                slot: tx_details.slot as u64,
                metadata: self.tx_metadata(tx_hash.as_str()).await,
            };
            res.push(info);
        }
        res
    }

    // Not retried on purpose: unlike a read, a resubmit is not idempotent.
    async fn submit_tx(&self, cbor_bytes: &[u8]) -> Result<(), Box<dyn std::error::Error>> {
        let result = self.0.transactions_submit(cbor_bytes.to_vec()).await?;
        trace!("TX submit result: {}", result);
        Ok(())
    }

    async fn chain_tip_slot_number(&self) -> Result<u64, Box<dyn std::error::Error>> {
        let slot = self
            .0
            .blocks_latest()
            .await?
            .slot
            .ok_or("Latest block carries no slot number")?;
        u64::try_from(slot).map_err(|_| format!("Negative chain tip slot number: {}", slot).into())
    }

    async fn wait_for_transaction_confirmation(
        &self,
        tx_id: TransactionHash,
    ) -> Result<(), Box<dyn std::error::Error>> {
        while self
            .0
            .transaction_by_hash(&tx_id.to_hex())
            .await
            .map(|_| ())
            .is_err()
        {
            tokio::time::sleep(std::time::Duration::from_secs(30)).await;
        }
        Ok(())
    }
}

/// The project_id held in a Blockfrost key file, checked against the configured network.
fn project_id_from_key_file(contents: &str, network_id: NetworkId) -> Result<String, Error> {
    // Trimmed, not just stripped of newlines: a stray space, tab or \r would otherwise survive
    // into the project_id and change which network the client talks to. Anything `trim` does
    // not cover — a UTF-8 BOM, quotes — is left in place on purpose and makes the prefix check
    // below fail closed rather than be guessed around.
    let project_id = contents.trim();
    validate_project_id(project_id, network_id)?;
    Ok(project_id.to_string())
}

/// Rejects a project_id issued for any network but the configured one.
///
/// The client derives its base URL from the project_id prefix alone and silently falls back to
/// MAINNET for anything it doesn't recognise, and the base URL cannot be overridden afterwards. So
/// the key has to be checked against the configured network right here, or a preprod agent would
/// happily read from and submit to mainnet.
fn validate_project_id(project_id: &str, network_id: NetworkId) -> Result<(), Error> {
    let expected_prefixes: &[&str] = match Network::from(network_id) {
        Mainnet => &[MAINNET_PREFIX],
        Preprod => &[PREPROD_PREFIX, PREVIEW_PREFIX],
    };
    if !expected_prefixes
        .iter()
        .any(|prefix| project_id.starts_with(prefix))
    {
        // Capped at the length of a network prefix. `take_while(is_ascii_alphabetic)` alone
        // would print the whole project_id for a key with no digits in it, and this message
        // reaches stdout and the log pipeline on every caller's `.expect`.
        const PREFIX_PREVIEW_LEN: usize = 7;
        let found_prefix: String = project_id
            .chars()
            .take(PREFIX_PREVIEW_LEN)
            .take_while(|c| c.is_ascii_alphabetic())
            .collect();
        return Err(Error::new(
            ErrorKind::InvalidData,
            format!(
                "Blockfrost project_id prefix '{}' does not match the configured network '{}' (expected one of {:?})",
                found_prefix,
                String::from(Network::from(network_id)),
                expected_prefixes
            ),
        ));
    }
    Ok(())
}

/// The body of a transaction, decoded from the hex-encoded CBOR the explorer serves.
fn decode_tx_body(tx_cbor: &str) -> Option<TransactionBody> {
    Some(
        Transaction::from_cbor_bytes(hex::decode(tx_cbor).ok()?.as_ref())
            .ok()?
            .body,
    )
}

/// Picks the output at `output_ix` out of a decoded transaction body.
///
/// The collateral return of a phase-2 failure is not in `outputs`; it is indexed right past the
/// last of them. Builders populate `collateral_return` on any TX that supplies collateral though,
/// so it only actually reaches the ledger when phase 2 failed — hand it back only once the TX is
/// confirmed invalid, or we would invent a UTxO that never existed. `valid_contract` resolves what
/// the explorer reports for the TX (`None` when that could not be fetched) and is only consulted
/// on this miss path, never on a normal lookup.
async fn select_output<F>(
    body: TransactionBody,
    output_ix: usize,
    valid_contract: impl FnOnce() -> F,
) -> Option<TransactionOutput>
where
    F: Future<Output = Option<bool>>,
{
    if output_ix == body.outputs.len() {
        let collateral_return = body.collateral_return?;
        return (!valid_contract().await?).then_some(collateral_return);
    }
    body.outputs.into_iter().nth(output_ix)
}

/// Rebuilds an output from the fields of a UTxO listing entry. The listing carries no script, so
/// the result never has one; see `Blockfrost::parse_blockfrost_output` for outputs that do.
fn output_from_listing(
    address: &str,
    output_amount: Vec<TxContentOutputAmountInner>,
    inline_datum: Option<String>,
    datum_hash: Option<String>,
) -> Option<TransactionOutput> {
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
        .and_then(|datum| hex::decode(datum).ok())
        .and_then(|datum_bytes| {
            PlutusData::from_cbor_bytes(datum_bytes.as_ref())
                .ok()
                .map(DatumOption::new_datum)
        })
        .or(datum_hash.and_then(|datum_hash| {
            DatumHash::from_hex(datum_hash.as_str())
                .ok()
                .map(DatumOption::new_hash)
        }));

    Some(TransactionOutput::new(
        Address::from_bech32(address).ok()?,
        value,
        datum,
        None,
    ))
}

/// Rebuilds the metadata of TX `tx_hash` from the per-label CBOR entries the explorer serves.
/// `None` when none of them could be decoded.
fn metadata_from_cbor_entries(tx_hash: &str, entries: Vec<TxContentMetadataCborInner>) -> Option<Metadata> {
    let mut metadata = Metadata::new();
    for entry in entries {
        // `cbor_metadata` is deprecated upstream in favour of `metadata`, and both are
        // nullable, so take whichever one this backend actually populated.
        let Some(raw_cbor) = entry.cbor_metadata.or(entry.metadata) else {
            warn!(
                "TX {}: metadata entry labelled '{}' carries no CBOR",
                tx_hash, entry.label
            );
            continue;
        };
        // Some responses render the CBOR in PostgreSQL bytea notation.
        let raw_hex = raw_cbor.strip_prefix("\\x").unwrap_or(raw_cbor.as_str());
        let decoded = entry.label.parse::<u64>().ok().and_then(|label| {
            let raw = hex::decode(raw_hex).ok()?;
            let metadatum = TransactionMetadatum::from_cbor_bytes(raw.as_ref()).ok()?;
            Some((label, unwrap_labelled_metadatum(label, metadatum)))
        });
        match decoded {
            Some((label, metadatum)) => metadata.set(label, metadatum),
            None => warn!(
                "TX {}: skipping undecodable metadata entry labelled '{}'",
                tx_hash, entry.label
            ),
        }
    }
    if metadata.is_empty() {
        None
    } else {
        Some(metadata)
    }
}

/// Blockfrost pagination starts from page 1, so the quotient is incremented. `None` for a zero
/// limit, and for the one offset whose page does not fit a `u32`.
fn blockfrost_page(offset: u32, limit: u16) -> Option<u32> {
    offset
        .checked_div(limit as u32)
        .and_then(|page| page.checked_add(1))
}

/// Strips the `{label: value}` envelope a metadata backend may wrap each metadatum in.
///
/// `cardano-db-sync` stores the CBOR of the singleton map keyed by the label rather than the bare
/// metadatum, and Blockfrost serves that verbatim. Rebuilding a `Metadata` from it unchanged would
/// nest every entry twice — `{4 -> {4 -> 1}}` instead of `{4 -> 1}` — and every consumer matching on
/// the metadatum type would silently see a `Map` where it expects an `Int` or `Bytes`. A backend
/// that serves the bare value is left untouched.
fn unwrap_labelled_metadatum(label: u64, datum: TransactionMetadatum) -> TransactionMetadatum {
    if let TransactionMetadatum::Map(map) = &datum {
        if let [(TransactionMetadatum::Int(Int::Uint { value, .. }), inner)] = &map.entries[..] {
            if *value == label {
                return inner.clone();
            }
        }
    }
    datum
}

pub struct UTxOInfo {
    pub utxo: TransactionUnspentOutput,
    pub slot: u64,
    /// The metadata of the transaction that produced this UTxO, if it carries any.
    pub metadata: Option<Metadata>,
}

pub enum AnyExplorer {
    Blockfrost(Blockfrost),
}

impl AnyExplorer {
    pub async fn new(config: &ExplorerConfig, network_id: NetworkId) -> Result<Self, Error> {
        match config {
            ExplorerConfig::BlockfrostKeyPath(blockfrost_key_path) => {
                Blockfrost::new(blockfrost_key_path, network_id)
                    .await
                    .map(AnyExplorer::Blockfrost)
            }
        }
    }
}

#[async_trait]
impl CardanoNetwork for AnyExplorer {
    async fn utxo_by_ref(&self, oref: OutputRef) -> Option<TransactionUnspentOutput> {
        match self {
            AnyExplorer::Blockfrost(blockfrost) => blockfrost.utxo_by_ref(oref).await,
        }
    }

    async fn utxos_by_pay_cred(
        &self,
        payment_credential: PaymentCredential,
        offset: u32,
        limit: u16,
    ) -> Vec<TransactionUnspentOutput> {
        match self {
            AnyExplorer::Blockfrost(blockfrost) => {
                blockfrost
                    .utxos_by_pay_cred(payment_credential, offset, limit)
                    .await
            }
        }
    }

    async fn utxos_by_address(
        &self,
        address: Address,
        offset: u32,
        limit: u16,
    ) -> Vec<TransactionUnspentOutput> {
        match self {
            AnyExplorer::Blockfrost(blockfrost) => blockfrost.utxos_by_address(address, offset, limit).await,
        }
    }
}

impl ExtendedCardanoNetwork for AnyExplorer {
    async fn slot_indexed_utxos_by_address(
        &self,
        address: Address,
        offset: u32,
        limit: u16,
    ) -> Vec<UTxOInfo> {
        match self {
            AnyExplorer::Blockfrost(blockfrost) => {
                blockfrost
                    .slot_indexed_utxos_by_address(address, offset, limit)
                    .await
            }
        }
    }

    async fn submit_tx(&self, cbor: &[u8]) -> Result<(), Box<dyn std::error::Error>> {
        match self {
            AnyExplorer::Blockfrost(blockfrost) => blockfrost.submit_tx(cbor).await,
        }
    }

    async fn chain_tip_slot_number(&self) -> Result<u64, Box<dyn std::error::Error>> {
        match self {
            AnyExplorer::Blockfrost(blockfrost) => blockfrost.chain_tip_slot_number().await,
        }
    }

    async fn wait_for_transaction_confirmation(
        &self,
        tx_id: TransactionHash,
    ) -> Result<(), Box<dyn std::error::Error>> {
        match self {
            AnyExplorer::Blockfrost(blockfrost) => blockfrost.wait_for_transaction_confirmation(tx_id).await,
        }
    }
}
