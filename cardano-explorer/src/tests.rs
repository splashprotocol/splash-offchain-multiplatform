//! Hermetic unit tests: no network, no environment (not even `TMPDIR`), no key files on disk beyond
//! one a single test writes next to its own executable and removes itself. The transaction fixtures
//! are the real on-chain reference transactions of the deployed mainnet validators, see
//! `resources/testdata/oldpools/manifest.tsv`.

use super::*;
use cml_chain::auxdata::MetadatumMap;
use cml_chain::crypto::hash::hash_transaction;
use cml_chain::plutus::Language;
use cml_chain::PolicyId;
use cml_core::serialization::Serialize;
use spectrum_cardano_lib::constants::CURRENCY_SYMBOL_HEX_STRING_LENGTH;
use spectrum_cardano_lib::{AssetClass, AssetName};
use std::cell::Cell;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

// -------------------------------------------------------------------------------------------------
// Fixtures: the reference-script transactions of the deployed validators
// -------------------------------------------------------------------------------------------------

/// One row of `resources/testdata/oldpools/manifest.tsv`.
struct ReferenceTx {
    name: &'static str,
    output_ix: usize,
    outputs: usize,
    epoch: u64,
    era: &'static str,
    script_hash: &'static str,
    tx_hash: &'static str,
    cbor_hex: &'static str,
}

const MANIFEST: &str = include_str!("../resources/testdata/oldpools/manifest.tsv");

/// First Conway epoch on mainnet; every earlier reference TX is Babbage-era.
const FIRST_CONWAY_EPOCH: u64 = 507;

fn fixture_cbor_hex(name: &str) -> &'static str {
    match name {
        "constFnPoolV1" => include_str!("../resources/testdata/oldpools/constFnPoolV1.cbor.hex"),
        "constFnPoolV2" => include_str!("../resources/testdata/oldpools/constFnPoolV2.cbor.hex"),
        "constFnPoolFeeSwitch" => {
            include_str!("../resources/testdata/oldpools/constFnPoolFeeSwitch.cbor.hex")
        }
        "balanceFnPoolV1" => include_str!("../resources/testdata/oldpools/balanceFnPoolV1.cbor.hex"),
        "stableFnPoolT2t" => include_str!("../resources/testdata/oldpools/stableFnPoolT2t.cbor.hex"),
        "limitOrder" => include_str!("../resources/testdata/oldpools/limitOrder.cbor.hex"),
        "royaltyPool" => include_str!("../resources/testdata/oldpools/royaltyPool.cbor.hex"),
        other => panic!(
            "manifest names fixture '{}' but no .cbor.hex is compiled in for it",
            other
        ),
    }
}

fn reference_txs() -> Vec<ReferenceTx> {
    let rows: Vec<ReferenceTx> = MANIFEST
        .lines()
        .filter(|line| !line.starts_with('#') && !line.trim().is_empty())
        .map(|line| {
            let cols: Vec<&'static str> = line.split('\t').collect();
            assert_eq!(cols.len(), 7, "malformed manifest row: {:?}", line);
            ReferenceTx {
                name: cols[0],
                output_ix: cols[1].parse().unwrap(),
                outputs: cols[2].parse().unwrap(),
                epoch: cols[3].parse().unwrap(),
                era: cols[4],
                script_hash: cols[5],
                tx_hash: cols[6],
                cbor_hex: fixture_cbor_hex(cols[0]),
            }
        })
        .collect();
    assert_eq!(rows.len(), 7, "one manifest row per deployed validator");
    rows
}

fn reference_tx(name: &str) -> ReferenceTx {
    reference_txs()
        .into_iter()
        .find(|tx| tx.name == name)
        .unwrap_or_else(|| panic!("no fixture named '{}'", name))
}

impl ReferenceTx {
    fn body(&self) -> TransactionBody {
        decode_tx_body(self.cbor_hex.trim())
            .unwrap_or_else(|| panic!("{}: transaction CBOR does not decode", self.name))
    }
}

/// Runs the output selection against a stubbed `valid_contract` lookup, also reporting how many
/// times that lookup was consulted.
fn select(
    body: TransactionBody,
    output_ix: usize,
    valid_contract: Option<bool>,
) -> (Option<TransactionOutput>, usize) {
    let lookups = Cell::new(0);
    let output = futures::executor::block_on(select_output(body, output_ix, || async {
        lookups.set(lookups.get() + 1);
        valid_contract
    }));
    (output, lookups.get())
}

// -------------------------------------------------------------------------------------------------
// A. Reference-script extraction from the real old-pool transactions
// -------------------------------------------------------------------------------------------------

fn assert_reference_script(name: &str) {
    let tx = reference_tx(name);
    let body = tx.body();
    assert_eq!(
        body.outputs.len(),
        tx.outputs,
        "{}: unexpected number of outputs",
        name
    );
    let (output, lookups) = select(body, tx.output_ix, None);
    let output = output.unwrap_or_else(|| panic!("{}: no output at index {}", name, tx.output_ix));
    assert_eq!(
        lookups, 0,
        "{}: a plain lookup must not consult valid_contract",
        name
    );
    let script = output
        .script_ref()
        .unwrap_or_else(|| panic!("{}: output {} carries no reference script", name, tx.output_ix));
    assert_eq!(
        script.hash().to_hex(),
        tx.script_hash,
        "{}: reference script hash",
        name
    );
    assert_eq!(
        script.language(),
        Some(Language::PlutusV2),
        "{}: deployed as PlutusV2",
        name
    );
}

#[test]
fn ref_script_const_fn_pool_v1() {
    assert_reference_script("constFnPoolV1")
}

#[test]
fn ref_script_const_fn_pool_v2() {
    assert_reference_script("constFnPoolV2")
}

#[test]
fn ref_script_const_fn_pool_fee_switch() {
    assert_reference_script("constFnPoolFeeSwitch")
}

#[test]
fn ref_script_balance_fn_pool_v1() {
    assert_reference_script("balanceFnPoolV1")
}

#[test]
fn ref_script_stable_fn_pool_t2t() {
    assert_reference_script("stableFnPoolT2t")
}

#[test]
fn ref_script_limit_order() {
    assert_reference_script("limitOrder")
}

#[test]
fn ref_script_royalty_pool() {
    assert_reference_script("royaltyPool")
}

#[test]
fn babbage_era_reference_txs_decode_with_the_conway_era_decoder() {
    // Reference scripts exist since Babbage, and every validator but the royalty pool was deployed
    // before Conway. Whether a Conway-capable cml still decodes those transactions, and still sees
    // their reference scripts, is exactly what this checks.
    //
    // The set of Babbage-era names is spelled out rather than counted, so a manifest typo (a wrong
    // epoch or era, a dropped row) is caught here instead of being absorbed into a shifted count.
    let (babbage, conway): (Vec<ReferenceTx>, Vec<ReferenceTx>) = reference_txs()
        .into_iter()
        .partition(|tx| tx.epoch < FIRST_CONWAY_EPOCH);
    let babbage_names: Vec<&str> = babbage.iter().map(|tx| tx.name).collect();
    assert_eq!(
        babbage_names,
        vec![
            "constFnPoolV1",
            "constFnPoolV2",
            "constFnPoolFeeSwitch",
            "balanceFnPoolV1",
            "stableFnPoolT2t",
            "limitOrder",
        ],
        "the Babbage-era deployments, in manifest order"
    );
    for tx in babbage {
        assert_eq!(tx.era, "babbage", "{}", tx.name);
        // The full check: the output decodes, carries a script, and it is the deployed one.
        assert_reference_script(tx.name);
    }
    let conway_names: Vec<&str> = conway.iter().map(|tx| tx.name).collect();
    assert_eq!(conway_names, vec!["royaltyPool"]);
    for tx in conway {
        assert_eq!(tx.era, "conway", "{}", tx.name);
    }
}

#[test]
fn fixtures_are_the_deployed_reference_txs_byte_for_byte() {
    // The deployment file pins each reference UTxO by transaction hash, so a fixture that is the
    // real transaction re-serialises to a body hashing to exactly that.
    for tx in reference_txs() {
        assert_eq!(
            hash_transaction(&tx.body()).to_hex(),
            tx.tx_hash,
            "{}: fixture is not the deployed reference transaction",
            tx.name
        );
    }
}

#[test]
fn royalty_pool_reference_output_carries_a_decodable_inline_datum() {
    let tx = reference_tx("royaltyPool");
    let output = tx.body().outputs.into_iter().nth(tx.output_ix).unwrap();
    let Some(DatumOption::Datum { datum, .. }) = output.datum() else {
        panic!("royaltyPool output 0 should carry an inline datum")
    };
    let reencoded = PlutusData::from_cbor_bytes(datum.to_cbor_bytes().as_ref()).unwrap();
    assert_eq!(reencoded, datum);
}

#[test]
fn undecodable_tx_cbor_is_a_miss() {
    assert!(decode_tx_body("").is_none());
    assert!(decode_tx_body("zz").is_none());
    assert!(decode_tx_body("00").is_none());
    let truncated = &reference_tx("constFnPoolV1").cbor_hex[..1000];
    assert!(decode_tx_body(truncated).is_none());
}

// --- the collateral-return rule -----------------------------------------------------------------

fn body_with_collateral_return() -> (TransactionBody, TransactionOutput) {
    let mut body = reference_tx("constFnPoolV1").body();
    let collateral_return = body.outputs.last().unwrap().clone();
    body.collateral_return = Some(collateral_return.clone());
    (body, collateral_return)
}

#[test]
fn regular_outputs_are_served_without_consulting_validity() {
    let tx = reference_tx("constFnPoolV1");
    let outputs = tx.body().outputs;
    for (ix, expected) in outputs.iter().enumerate() {
        let (output, lookups) = select(tx.body(), ix, Some(false));
        assert_eq!(output.as_ref(), Some(expected), "output {}", ix);
        assert_eq!(lookups, 0, "output {}", ix);
    }
}

#[test]
fn index_past_the_last_output_is_a_miss_without_a_collateral_return() {
    let body = reference_tx("constFnPoolV1").body();
    assert!(body.collateral_return.is_none(), "fixture assumption");
    let len = body.outputs.len();
    let (output, lookups) = select(body, len, Some(false));
    assert!(output.is_none(), "nothing may be fabricated past the last output");
    assert_eq!(
        lookups, 0,
        "no collateral return, so validity is not even looked up"
    );
}

#[test]
fn collateral_return_is_served_only_for_a_confirmed_phase2_failure() {
    let (body, collateral_return) = body_with_collateral_return();
    let len = body.outputs.len();
    let (output, lookups) = select(body, len, Some(false));
    assert_eq!(output, Some(collateral_return));
    assert_eq!(lookups, 1);
}

#[test]
fn collateral_return_is_withheld_while_the_tx_is_valid() {
    let (body, _) = body_with_collateral_return();
    let len = body.outputs.len();
    let (output, lookups) = select(body, len, Some(true));
    assert!(output.is_none());
    assert_eq!(lookups, 1);
}

#[test]
fn collateral_return_is_withheld_when_validity_cannot_be_fetched() {
    let (body, _) = body_with_collateral_return();
    let len = body.outputs.len();
    let (output, lookups) = select(body, len, None);
    assert!(output.is_none());
    assert_eq!(lookups, 1);
}

#[test]
fn index_beyond_the_collateral_return_slot_is_a_plain_miss() {
    let (body, _) = body_with_collateral_return();
    let len = body.outputs.len();
    let (output, lookups) = select(body, len + 1, Some(false));
    assert!(output.is_none());
    assert_eq!(lookups, 0);
}

// -------------------------------------------------------------------------------------------------
// B. Rebuilding an output from a UTxO listing entry
// -------------------------------------------------------------------------------------------------

/// The CIP-19 example base address.
const ADDRESS: &str =
    "addr1qx2fxv2umyhttkxyxp8x0dlpdt3k6cwng5pxj3jhsydzer3n0d3vllmyqwsx5wktcd8cc3sq835lu7drv2xwl2wywfgse35a3x";
const DATUM_HASH: &str = "9e478573ab81ea7a8e31891ce0648b81229f408d596a3483e6f4f9b92d3cf710";
/// `Constr 0 []`.
const INLINE_DATUM_HEX: &str = "d87980";

fn amount(unit: &str, quantity: &str) -> TxContentOutputAmountInner {
    TxContentOutputAmountInner::new(unit.to_string(), quantity.to_string())
}

/// A policy id made of one repeated byte.
fn policy(byte_hex: &str) -> String {
    byte_hex.repeat(CURRENCY_SYMBOL_HEX_STRING_LENGTH / 2)
}

/// A Blockfrost `unit`: policy id and asset name hex, concatenated.
fn unit(policy: &str, asset_name_hex: &str) -> String {
    format!("{}{}", policy, asset_name_hex)
}

fn token(policy: &str, asset_name_hex: &str) -> AssetClass {
    Token(RawToken(
        PolicyId::from_hex(policy).unwrap(),
        AssetName::try_from_hex(asset_name_hex).unwrap(),
    ))
}

#[test]
fn listing_test_constants_are_well_formed() {
    assert!(Address::from_bech32(ADDRESS).is_ok());
    assert!(DatumHash::from_hex(DATUM_HASH).is_ok());
    assert_eq!(policy("aa").len(), CURRENCY_SYMBOL_HEX_STRING_LENGTH);
}

#[test]
fn listing_lovelace_only_output() {
    let output = output_from_listing(ADDRESS, vec![amount(LOVELACE, "2000000")], None, None).unwrap();
    assert_eq!(output.address(), &Address::from_bech32(ADDRESS).unwrap());
    assert_eq!(output.amount(), &Value::from(2_000_000u64));
    assert!(output.datum().is_none());
    assert!(
        output.script_ref().is_none(),
        "the listing path never yields a script"
    );
}

#[test]
fn listing_native_assets_are_split_into_policy_and_asset_name() {
    let snek = unit(&policy("aa"), "534e454b");
    let nameless = unit(&policy("bb"), "");
    let output = output_from_listing(
        ADDRESS,
        vec![
            amount(LOVELACE, "1500000"),
            amount(&snek, "7"),
            amount(&nameless, "1"),
        ],
        None,
        None,
    )
    .unwrap();
    let mut expected = Value::from(1_500_000u64);
    expected.add_unsafe(token(&policy("aa"), "534e454b"), 7);
    expected.add_unsafe(token(&policy("bb"), ""), 1);
    assert_eq!(output.amount(), &expected);
    assert_eq!(
        output.amount().amount_of(token(&policy("aa"), "534e454b")),
        Some(7)
    );
    assert_eq!(output.amount().amount_of(token(&policy("bb"), "")), Some(1));
}

#[test]
fn listing_empty_amount_is_a_zero_value() {
    let output = output_from_listing(ADDRESS, vec![], None, None).unwrap();
    assert_eq!(output.amount(), &Value::zero());
}

#[test]
fn listing_datum_hash_only_yields_a_hash_datum() {
    // Pre-Babbage pools carry their datum by hash.
    let output = output_from_listing(
        ADDRESS,
        vec![amount(LOVELACE, "1")],
        None,
        Some(DATUM_HASH.to_string()),
    )
    .unwrap();
    let hash = DatumHash::from_hex(DATUM_HASH).unwrap();
    assert_eq!(output.datum(), Some(DatumOption::new_hash(hash)));
    assert_eq!(output.datum_hash(), Some(&hash));
}

#[test]
fn listing_inline_datum_is_hex_decoded_then_cbor_parsed() {
    let output = output_from_listing(ADDRESS, vec![], Some(INLINE_DATUM_HEX.to_string()), None).unwrap();
    let Some(DatumOption::Datum { datum, .. }) = output.datum() else {
        panic!("expected an inline datum")
    };
    let expected = PlutusData::from_cbor_bytes(hex::decode(INLINE_DATUM_HEX).unwrap().as_ref()).unwrap();
    assert_eq!(datum, expected);
    assert!(
        matches!(&datum, PlutusData::ConstrPlutusData(constr) if constr.alternative == 0 && constr.fields.is_empty()),
        "the bytes mean `Constr 0 []`, got {:?}",
        datum
    );
    // Fed as text instead of decoded, the same characters are not that datum.
    assert_ne!(
        PlutusData::from_cbor_bytes(INLINE_DATUM_HEX.as_bytes()).ok(),
        Some(expected)
    );
}

#[test]
fn listing_inline_datum_equals_the_one_decoded_from_the_tx_cbor() {
    // The listing serves the datum the CBOR path decodes, as hex. Both paths must agree on it.
    let tx = reference_tx("royaltyPool");
    let from_cbor = tx.body().outputs.into_iter().nth(tx.output_ix).unwrap();
    let Some(DatumOption::Datum { datum, .. }) = from_cbor.datum() else {
        panic!("fixture assumption: inline datum")
    };
    let inline_hex = hex::encode(datum.to_cbor_bytes());
    let address = from_cbor.address().to_bech32(None).unwrap();
    let from_listing = output_from_listing(address.as_str(), vec![], Some(inline_hex), None).unwrap();
    assert_eq!(from_listing.datum(), from_cbor.datum());
    assert_eq!(from_listing.address(), from_cbor.address());
}

#[test]
fn listing_inline_datum_wins_over_the_datum_hash() {
    let output = output_from_listing(
        ADDRESS,
        vec![],
        Some(INLINE_DATUM_HEX.to_string()),
        Some(DATUM_HASH.to_string()),
    )
    .unwrap();
    assert!(matches!(output.datum(), Some(DatumOption::Datum { .. })));
}

#[test]
fn listing_malformed_inline_datum_hex_falls_back_to_the_hash() {
    let output = output_from_listing(
        ADDRESS,
        vec![],
        Some("not-hex".to_string()),
        Some(DATUM_HASH.to_string()),
    )
    .unwrap();
    assert_eq!(
        output.datum_hash(),
        Some(&DatumHash::from_hex(DATUM_HASH).unwrap())
    );
}

#[test]
fn listing_inline_datum_with_invalid_cbor_falls_back_to_the_hash() {
    // Valid hex, but a lone CBOR break marker is not a datum.
    let output = output_from_listing(
        ADDRESS,
        vec![],
        Some("ff".to_string()),
        Some(DATUM_HASH.to_string()),
    )
    .unwrap();
    assert!(matches!(output.datum(), Some(DatumOption::Hash { .. })));
}

#[test]
fn listing_undecodable_datum_without_a_usable_hash_yields_a_datumless_output() {
    // Pinned, not endorsed: the datum is dropped rather than the whole output being refused.
    let output = output_from_listing(ADDRESS, vec![], Some("not-hex".to_string()), None).unwrap();
    assert!(output.datum().is_none());
    let output = output_from_listing(ADDRESS, vec![], None, Some("too-short".to_string())).unwrap();
    assert!(output.datum().is_none());
}

#[test]
fn listing_non_numeric_quantity_skips_that_asset_only() {
    let output = output_from_listing(
        ADDRESS,
        vec![
            amount(LOVELACE, "2000000"),
            amount(&unit(&policy("aa"), "01"), "abc"),
            amount(&unit(&policy("bb"), "02"), "-5"),
            amount(&unit(&policy("cc"), "03"), "18446744073709551616"), // u64::MAX + 1
            amount(&unit(&policy("dd"), "04"), "5"),
        ],
        None,
        None,
    )
    .unwrap();
    assert_eq!(output.amount().coin, 2_000_000);
    assert_eq!(output.amount().amount_of(token(&policy("aa"), "01")), None);
    assert_eq!(output.amount().amount_of(token(&policy("bb"), "02")), None);
    assert_eq!(output.amount().amount_of(token(&policy("cc"), "03")), None);
    assert_eq!(output.amount().amount_of(token(&policy("dd"), "04")), Some(5));
}

#[test]
fn listing_non_numeric_lovelace_leaves_the_coin_at_zero() {
    let output = output_from_listing(ADDRESS, vec![amount(LOVELACE, "lots")], None, None).unwrap();
    assert_eq!(output.amount(), &Value::zero());
}

#[test]
fn listing_unit_that_does_not_parse_as_a_token_is_skipped() {
    // Policy-id sized but not hex, and a policy id followed by a 33-byte asset name.
    let not_hex = policy("zz");
    let name_too_long = unit(&policy("aa"), &"01".repeat(33));
    let output = output_from_listing(
        ADDRESS,
        vec![
            amount(LOVELACE, "1"),
            amount(&not_hex, "9"),
            amount(&name_too_long, "9"),
        ],
        None,
        None,
    )
    .unwrap();
    assert_eq!(output.amount(), &Value::from(1u64));
}

#[test]
#[ignore = "BUG (spectrum_cardano_lib::Token::try_from_raw_string): `str::split_at(56)` panics on a unit \
            shorter than a policy id, so one malformed `unit` in a listing takes the whole task down \
            instead of being skipped like every other unparseable asset"]
fn listing_unit_shorter_than_a_policy_id_is_skipped_not_a_panic() {
    let output = output_from_listing(
        ADDRESS,
        vec![amount(LOVELACE, "1"), amount("ada", "9")],
        None,
        None,
    )
    .unwrap();
    assert_eq!(output.amount(), &Value::from(1u64));
}

#[test]
fn listing_bad_bech32_address_is_a_miss() {
    assert!(output_from_listing("addr1notanaddress", vec![amount(LOVELACE, "1")], None, None).is_none());
    assert!(output_from_listing("", vec![], None, None).is_none());
    let hex_not_bech32 = reference_tx("constFnPoolV1").body().outputs[0].address().to_hex();
    assert!(output_from_listing(hex_not_bech32.as_str(), vec![], None, None).is_none());
}

// -------------------------------------------------------------------------------------------------
// C. Metadata
// -------------------------------------------------------------------------------------------------

const TX: &str = "0000000000000000000000000000000000000000000000000000000000000000";

fn row(label: &str, cbor_metadata: Option<&str>, metadata: Option<&str>) -> TxContentMetadataCborInner {
    TxContentMetadataCborInner::new(
        label.to_string(),
        cbor_metadata.map(str::to_string),
        metadata.map(str::to_string),
    )
}

fn int(value: u64) -> TransactionMetadatum {
    TransactionMetadatum::new_int(Int::new_uint(value))
}

fn metadatum(cbor_hex: &str) -> TransactionMetadatum {
    TransactionMetadatum::from_cbor_bytes(hex::decode(cbor_hex).unwrap().as_ref()).unwrap()
}

#[test]
fn metadata_label_envelope_is_unwrapped() {
    // {4: 1}, as cardano-db-sync stores it.
    let metadata = metadata_from_cbor_entries(TX, vec![row("4", Some("a10401"), None)]).unwrap();
    assert_eq!(metadata.len(), 1);
    assert_eq!(metadata.get(4), Some(&int(1)), "an Int, not the enveloping Map");
}

#[test]
fn metadata_bare_value_passes_through() {
    let metadata = metadata_from_cbor_entries(TX, vec![row("4", Some("01"), None)]).unwrap();
    assert_eq!(metadata.get(4), Some(&int(1)));
}

#[test]
fn metadata_map_keyed_by_another_label_is_not_unwrapped() {
    // {5: 1} filed under label 4.
    let metadata = metadata_from_cbor_entries(TX, vec![row("4", Some("a10501"), None)]).unwrap();
    let mut expected = MetadatumMap::new();
    expected.set(int(5), int(1));
    assert_eq!(metadata.get(4), Some(&TransactionMetadatum::new_map(expected)));
}

#[test]
fn unwrap_leaves_non_singleton_and_non_uint_keyed_maps_alone() {
    // {4: 1, 5: 2}
    let two_entries = metadatum("a204010502");
    assert_eq!(unwrap_labelled_metadatum(4, two_entries.clone()), two_entries);
    // {-4: 1}: a negative key is an Nint, never the label
    let negative_key = metadatum("a12301");
    assert_eq!(unwrap_labelled_metadatum(4, negative_key.clone()), negative_key);
    // {}
    let empty = metadatum("a0");
    assert_eq!(unwrap_labelled_metadatum(4, empty.clone()), empty);
    // {4: {4: 1}}: only one envelope is stripped
    assert_eq!(
        unwrap_labelled_metadatum(4, metadatum("a104a10401")),
        metadatum("a10401")
    );
}

#[test]
fn metadata_postgres_bytea_prefix_is_stripped() {
    let metadata = metadata_from_cbor_entries(TX, vec![row("4", Some("\\xa10401"), None)]).unwrap();
    assert_eq!(metadata.get(4), Some(&int(1)));
}

#[test]
fn metadata_field_is_used_when_cbor_metadata_is_null() {
    let metadata = metadata_from_cbor_entries(TX, vec![row("4", None, Some("a10401"))]).unwrap();
    assert_eq!(metadata.get(4), Some(&int(1)));
}

#[test]
fn metadata_cbor_metadata_takes_precedence_over_metadata() {
    let metadata = metadata_from_cbor_entries(TX, vec![row("4", Some("a10401"), Some("a10402"))]).unwrap();
    assert_eq!(metadata.get(4), Some(&int(1)));
}

#[test]
fn metadata_entry_without_any_cbor_is_skipped() {
    assert!(metadata_from_cbor_entries(TX, vec![row("4", None, None)]).is_none());
    let metadata =
        metadata_from_cbor_entries(TX, vec![row("4", None, None), row("5", Some("a10501"), None)]).unwrap();
    assert_eq!(metadata.len(), 1);
    assert!(metadata.get(4).is_none());
    assert_eq!(metadata.get(5), Some(&int(1)));
}

#[test]
fn metadata_unparseable_label_is_skipped() {
    assert!(metadata_from_cbor_entries(TX, vec![row("four", Some("a10401"), None)]).is_none());
    assert!(metadata_from_cbor_entries(TX, vec![row("-4", Some("a10401"), None)]).is_none());
    assert!(metadata_from_cbor_entries(TX, vec![row("", Some("a10401"), None)]).is_none());
}

#[test]
fn metadata_undecodable_cbor_is_skipped() {
    assert!(metadata_from_cbor_entries(TX, vec![row("4", Some("zz"), None)]).is_none());
    assert!(metadata_from_cbor_entries(TX, vec![row("4", Some("ff"), None)]).is_none());
    assert!(metadata_from_cbor_entries(TX, vec![row("4", Some(""), None)]).is_none());
}

#[test]
fn metadata_all_entries_skipped_or_empty_listing_is_none() {
    assert!(metadata_from_cbor_entries(TX, vec![]).is_none());
    assert!(metadata_from_cbor_entries(
        TX,
        vec![row("x", Some("a10401"), None), row("4", Some("zz"), None)]
    )
    .is_none());
}

#[test]
fn metadata_proxy_order_shape_has_bytes_under_0_to_3_and_an_int_under_4() {
    // The shape splash-dao-offchain's get_proxy_order_metadata expects: byte strings under labels
    // 0..=3 and an integer under 4, each served in its label envelope.
    let rows = vec![
        row("0", Some("a1004401020304"), None),
        row("1", Some("a1014405060708"), None),
        row("2", Some("a10244090a0b0c"), None),
        row("3", Some("a103440d0e0f10"), None),
        row("4", Some("a1041a000f4240"), None),
    ];
    let metadata = metadata_from_cbor_entries(TX, rows).unwrap();
    assert_eq!(metadata.len(), 5);
    for (label, first_byte) in [(0u64, 1u8), (1, 5), (2, 9), (3, 13)] {
        match metadata.get(label) {
            Some(TransactionMetadatum::Bytes { bytes, .. }) => {
                assert_eq!(
                    bytes,
                    &vec![first_byte, first_byte + 1, first_byte + 2, first_byte + 3]
                )
            }
            other => panic!("label {}: expected Bytes, got {:?}", label, other),
        }
    }
    assert_eq!(metadata.get(4), Some(&int(1_000_000)));
}

// -------------------------------------------------------------------------------------------------
// D. Network interlock on the project_id
// -------------------------------------------------------------------------------------------------

#[test]
fn network_id_maps_to_the_client_prefix() {
    assert!(matches!(Network::from(NetworkId::MAINNET), Mainnet));
    assert!(matches!(Network::from(NetworkId::PREPROD), Preprod));
    assert_eq!(String::from(Network::from(NetworkId::MAINNET)), MAINNET_PREFIX);
    assert_eq!(String::from(Network::from(NetworkId::PREPROD)), PREPROD_PREFIX);
}

#[test]
fn project_id_mainnet_key_for_mainnet_is_accepted() {
    assert!(validate_project_id("mainnetAbCdEf0123456789", NetworkId::MAINNET).is_ok());
}

#[test]
fn project_id_preprod_and_preview_keys_for_preprod_are_accepted() {
    assert!(validate_project_id("preprodAbCdEf0123456789", NetworkId::PREPROD).is_ok());
    assert!(validate_project_id("previewAbCdEf0123456789", NetworkId::PREPROD).is_ok());
}

#[test]
fn project_id_preprod_key_for_mainnet_is_rejected() {
    let err = validate_project_id("preprodAbCdEf0123456789", NetworkId::MAINNET).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::InvalidData);
    let message = err.to_string();
    assert!(message.contains("prefix 'preprod'"), "{}", message);
    assert!(message.contains("network 'mainnet'"), "{}", message);
    assert!(message.contains(r#"["mainnet"]"#), "{}", message);
}

#[test]
fn project_id_mainnet_key_for_preprod_is_rejected() {
    let err = validate_project_id("mainnetAbCdEf0123456789", NetworkId::PREPROD).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::InvalidData);
    let message = err.to_string();
    assert!(message.contains("prefix 'mainnet'"), "{}", message);
    assert!(message.contains("network 'preprod'"), "{}", message);
    assert!(message.contains(r#"["preprod", "preview"]"#), "{}", message);
}

#[test]
fn project_id_without_a_network_prefix_is_rejected_on_both_networks() {
    for project_id in ["", "1234567890abcdef", "MAINNETabc", "main-net", "testnetAbC"] {
        assert!(
            validate_project_id(project_id, NetworkId::MAINNET).is_err(),
            "{:?}",
            project_id
        );
        assert!(
            validate_project_id(project_id, NetworkId::PREPROD).is_err(),
            "{:?}",
            project_id
        );
    }
}

#[test]
fn project_id_error_never_leaks_more_than_a_seven_char_prefix_preview() {
    let key = "abcdefghijklmnopqrstuvwxyz0123456789";
    let message = validate_project_id(key, NetworkId::MAINNET)
        .unwrap_err()
        .to_string();
    assert!(message.contains("prefix 'abcdefg'"), "{}", message);
    assert!(!message.contains("abcdefgh"), "{}", message);
    assert!(!message.contains(key), "{}", message);

    // The preview stops at the first non-letter, and at nothing for a key with none up front.
    let message = validate_project_id("pre9rodSecret", NetworkId::MAINNET)
        .unwrap_err()
        .to_string();
    assert!(message.contains("prefix 'pre'"), "{}", message);
    assert!(!message.contains("Secret"), "{}", message);
    let message = validate_project_id("9secretSecret", NetworkId::MAINNET)
        .unwrap_err()
        .to_string();
    assert!(message.contains("prefix ''"), "{}", message);
    assert!(!message.contains("ecret"), "{}", message);
}

#[test]
fn project_id_is_trimmed_before_the_check() {
    assert_eq!(
        project_id_from_key_file("  mainnetKEY\r\n", NetworkId::MAINNET).unwrap(),
        "mainnetKEY"
    );
    assert_eq!(
        project_id_from_key_file("\n\tpreviewKEY \n\n", NetworkId::PREPROD).unwrap(),
        "previewKEY"
    );
    assert_eq!(
        project_id_from_key_file("preprodKEY", NetworkId::PREPROD).unwrap(),
        "preprodKEY"
    );
    assert!(project_id_from_key_file(" \r\n", NetworkId::MAINNET).is_err());
}

#[test]
fn project_id_with_a_utf8_bom_is_rejected_rather_than_trimmed() {
    // `str::trim` strips Unicode White_Space only, and U+FEFF is not one. A key file saved with a
    // BOM therefore fails closed at start-up (prefix preview '') instead of being accepted.
    let err = project_id_from_key_file("\u{feff}mainnetKEY", NetworkId::MAINNET).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::InvalidData);
    assert!(err.to_string().contains("prefix ''"), "{}", err);
}

/// A key file written next to the test executable (the cargo build output directory, which cargo
/// has just written to, so it exists and is writable whatever `TMPDIR` says), removed on drop.
///
/// `Blockfrost::new` is `fs::read_to_string` followed by the pure, separately tested
/// `project_id_from_key_file`, so this is the one test that has to touch a real file.
struct KeyFile(PathBuf);

impl KeyFile {
    fn write(contents: &str) -> Self {
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let scratch = std::env::current_exe()
            .ok()
            .and_then(|exe| exe.parent().map(Path::to_path_buf))
            .expect("the test executable has a parent directory");
        let path = scratch.join(format!(
            "cardano-explorer-test-{}-{}.key",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::write(&path, contents)
            .unwrap_or_else(|err| panic!("cannot write scratch key file {}: {}", path.display(), err));
        KeyFile(path)
    }
}

impl Drop for KeyFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

#[tokio::test]
async fn new_reads_trims_and_checks_the_key_file() {
    let key = KeyFile::write("  mainnetTESTKEY0123456789\r\n");
    assert!(Blockfrost::new(&key.0, NetworkId::MAINNET).await.is_ok());
    let Err(err) = Blockfrost::new(&key.0, NetworkId::PREPROD).await else {
        panic!("a mainnet key must not be accepted for preprod")
    };
    assert_eq!(err.kind(), ErrorKind::InvalidData);
    assert!(err.to_string().contains("prefix 'mainnet'"), "{}", err);
}

#[tokio::test]
async fn new_fails_on_a_missing_key_file() {
    // Nothing is written: the path sits under a directory that does not exist, in the crate's own
    // (committed, read-only for tests) fixture tree rather than anywhere `TMPDIR` points.
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("resources/testdata/no-such-directory")
        .join(format!(
            "cardano-explorer-test-missing-{}.key",
            std::process::id()
        ));
    assert!(!path.parent().unwrap().exists(), "fixture assumption");
    let Err(err) = Blockfrost::new(&path, NetworkId::MAINNET).await else {
        panic!("a missing key file must not yield a client")
    };
    assert_eq!(err.kind(), ErrorKind::NotFound);
}

// -------------------------------------------------------------------------------------------------
// E. retry!
// -------------------------------------------------------------------------------------------------

#[tokio::test]
async fn retry_returns_the_first_some_and_stops() {
    let mut calls = 0;
    let result = retry!({
        calls += 1;
        if calls < 3 {
            None
        } else {
            Some(calls)
        }
    });
    assert_eq!(result, Some(3));
    assert_eq!(calls, 3);
}

#[tokio::test]
async fn retry_evaluates_once_when_the_first_attempt_succeeds() {
    let mut calls = 0;
    let result = retry!(
        {
            calls += 1;
            Some(calls)
        },
        5,
        1
    );
    assert_eq!(result, Some(1));
    assert_eq!(calls, 1);
}

#[tokio::test]
async fn retry_gives_up_after_count_plus_two_attempts() {
    // Off by one, pinned: `retries > $count` is only checked after an attempt has failed, so
    // `$count` bounds the number of *re*tries at count + 1 and an always-failing expression is
    // evaluated count + 2 times in total (27 for the default arity of 25).
    let mut calls = 0;
    let result = retry!(
        {
            calls += 1;
            None::<u8>
        },
        2,
        1
    );
    assert_eq!(result, None);
    assert_eq!(calls, 4);

    let mut calls = 0;
    let result = retry!(
        {
            calls += 1;
            None::<u8>
        },
        0,
        1
    );
    assert_eq!(result, None);
    assert_eq!(calls, 2, "even a zero count retries once");
}

// -------------------------------------------------------------------------------------------------
// F. Page math
// -------------------------------------------------------------------------------------------------

#[test]
fn page_math_starts_at_page_one_and_steps_per_limit() {
    assert_eq!(blockfrost_page(0, 50), Some(1));
    assert_eq!(blockfrost_page(50, 50), Some(2));
    assert_eq!(blockfrost_page(100, 50), Some(3));
    assert_eq!(blockfrost_page(0, 1), Some(1));
    assert_eq!(blockfrost_page(7, 1), Some(8));
}

#[test]
fn page_math_rounds_a_non_multiple_offset_down() {
    // Known trap, pinned: an offset that is not a multiple of the limit lands on the page that
    // contains it, so the first `offset % limit` UTxOs of that page are served a second time.
    assert_eq!(blockfrost_page(25, 50), Some(1));
    assert_eq!(blockfrost_page(49, 50), Some(1));
    assert_eq!(blockfrost_page(75, 50), Some(2));
}

#[test]
fn page_math_zero_limit_is_none() {
    assert_eq!(blockfrost_page(0, 0), None);
    assert_eq!(blockfrost_page(100, 0), None);
}

#[test]
fn page_math_does_not_overflow_at_the_top_of_the_offset_range() {
    // Unreachable from a real listing, but the increment past the quotient must not panic (debug)
    // or wrap to page 0 (release): a page that cannot be represented is `None` like a zero limit.
    assert_eq!(blockfrost_page(u32::MAX, 1), None);
    assert_eq!(blockfrost_page(u32::MAX - 1, 1), Some(u32::MAX));
    assert_eq!(blockfrost_page(u32::MAX, 2), Some(u32::MAX / 2 + 1));
    assert_eq!(
        blockfrost_page(u32::MAX, u16::MAX),
        Some(u32::MAX / u16::MAX as u32 + 1)
    );
}
