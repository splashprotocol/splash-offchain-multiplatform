use cml_chain::{address::Address, builders::tx_builder::TransactionUnspentOutput, plutus::PlutusData, PolicyId, Script, transaction::{TransactionInput, TransactionOutput}, Value};
use cml_chain::assets::{AssetName, MultiAsset};
use cml_chain::builders::input_builder::SingleInputBuilder;
use cml_chain::builders::output_builder::SingleOutputBuilderResult;
use cml_chain::builders::redeemer_builder::RedeemerWitnessKey;
use cml_chain::builders::tx_builder::ChangeSelectionAlgo;
use cml_chain::builders::witness_builder::{PartialPlutusWitness, PlutusScriptWitness};
use cml_chain::plutus::{PlutusV2Script, RedeemerTag};
use cml_chain::transaction::DatumOption;
use cml_chain::utils::BigInt;
use cml_core::serialization::{Deserialize, FromBytes};
use cml_crypto::{DatumHash, ScriptHash, TransactionHash};
use cml_crypto::chain_core::property::FromStr;
use cml_core::serialization::Serialize;

use cardano_explorer::client::Explorer;
use cardano_explorer::data::ExplorerConfig;
use cardano_explorer::data::full_tx_out::ExplorerTxOut;
use cardano_explorer::data::value::ExplorerValue;
use spectrum_cardano_lib::hash::hash_transaction_canonical;
use spectrum_cardano_lib::OutputRef;
use spectrum_cardano_lib::protocol_params::constant_tx_builder;
use spectrum_cardano_lib::transaction::TransactionOutputExtension;
use spectrum_offchain_cardano::constants::POOL_EXECUTION_UNITS;

#[tokio::main]
async fn main() {
    let mut tx_builder = constant_tx_builder();
    let explorer = Explorer::new(ExplorerConfig {
        url: "https://explorer.spectrum.fi",
    });

    let collateral_1 = to_cml_utxo(explorer.get_utxo(OutputRef::new(
        TransactionHash::from_hex(
            "7aca5a6429fa775bfeb2a0c7b704e24ff63cee60d5ed061c5bac8c529dd6479d").unwrap(),
        0,
    )).await.unwrap());

    // let collateral_2 = to_cml_utxo(explorer.get_utxo(OutputRef::new(
    //     TransactionHash::from_hex(
    //         "bfecffaf09bb9a078cd4d56b1db9d3eff7d99b3fa290c60ceb74f9110a3d2545").unwrap(),
    //     2,
    // )).await.unwrap());

    let user_utxo: TransactionUnspentOutput = to_cml_utxo(explorer.get_utxo(OutputRef::new(
        TransactionHash::from_hex(
            "26068b7ec361d2c1a4940ec5bf836569cfa6dbbafd111505e8dc530b773d0cc5").unwrap(),
        0,
    )).await.unwrap());

    let unlock_ref_utxo = to_cml_utxo(explorer.get_utxo(OutputRef::new(
        TransactionHash::from_hex(
            "27cd7f14a124e2078de83e99a97cc52855b457845c4fc2cad365b931908f7e42").unwrap(),
        0,
    )).await.unwrap());

    let mut unlock_utxo_multi_asset = MultiAsset::new();
    unlock_utxo_multi_asset
        .set(
            PolicyId::from_hex("449b78b977de3bd0bdf66d58d9e28e14762aad9cf1a80c2c3d2e7912").unwrap(),
            AssetName::try_from("LQ_ADA_LQ").unwrap(),
            51178
        );
    let mut unlock_utxo_value = Value::new(1400750, unlock_utxo_multi_asset.clone());

    let unlock_utxo_info = TransactionOutput::new(
        Address::from_bech32("addr1z8wma0rzvdexhnqrty6t8dcur7c5ffu2rjau2ayec3d3az2f2sak7tcj4wal6pm37x3hsrlwtwrk3s26sddscfuh6fnqrnxmhx").unwrap(),
        unlock_utxo_value.clone(),
        Some(DatumOption::new_datum(PlutusData::from_cbor_bytes(hex::decode("d8799f1b0000018d9c7e5da2581c7c1c952fd498443de35e4df37dd01b1ec5b010aa8c5bbd08e09b9c1eff").unwrap().as_ref()).unwrap())),
        Some(Script::new_plutus_v2(PlutusV2Script::new(
            hex::decode("5903f6010000323232323232323232222323232533300932323232533300d3370e90011806000899191919299980899b8748000c0400044c8c8c8c8c8c94ccc05ccdc3a4000602c002264646464a66603600220042940cc88c8cc00400400c894ccc08400452809919299981019b8f00200514a2266008008002604a0046eb8c08c004dd6180f98101810181018101810181018101810180c1805980c00a9bae3007301801753330193375e6014602e00a6014602e002264a66603466e1d2004301900113232323232325333020002100114a066e24dd69808180e80e1bad3010301d00432323300100100222533302400114a226464a666046646464646466e24004c8c8c94ccc0accdc3a40040022900009bad30303029002302900132533302a3370e90010008a60103d87a8000132323300100100222533303000114c103d87a800013232323253330313371e018004266e95200033035375000297ae0133006006003375a60640066eb8c0c0008c0d0008c0c8004dd598179814001181400099198008008061129998168008a6103d87a8000132323232533302e3371e016004266e95200033032374c00297ae01330060060033756605e0066eb8c0b4008c0c4008c0bc004dd6981600098160011bae302a001302a003375c60500042660080080022940c0a0008dd618130009919198008008011129998120008a5eb804c8ccc888c8cc00400400c894ccc0a8004400c4c8cc0b0dd3998161ba90063302c37526eb8c0a4004cc0b0dd41bad302a0014bd7019801801981700118160009bae302300137566048002660060066050004604c0026eacc02cc070028dd59805180d802980b0009810000980c0008b1802180b800899911919299980e99b87480080044c8c94ccc07ccdc3a400460406ea8c030c074c040c0740184cdc4002800899b89005001375a604600260360042940c06c004c030c064c030c064008c078c07cc07cc07cc07cc07cc07cc07cc05c03cdd69805180b80b180e800980a8008b19991800800911299980e0010a6103d87a800013232533301b3370e0069000099ba548000cc07c0092f5c0266600a00a00266e0400d20023020003301e00237586002602801801c460366038603800260026024004460326034002602e002601e0022c64646600200200444a66602c0022980103d87a80001323253330153375e600c602600400e266e952000330190024bd70099802002000980d001180c0009bac3001300e006230150013013001300b0011630110013011002300f001300700414984d958dd68021800802119299980419b87480000044c8c8c8c94ccc03cc04800852616375c602000260200046eb4c038004c01800858c0180048c014dd5000918019baa0015734aae7555cf2ab9f5740ae855d11").unwrap(),
        ))),
    );
    let unlock_utxo_output = TransactionOutput::new(
        Address::from_bech32("addr1q97pe9f06jvyg00rtexlxlwsrv0vtvqs42x9h0gguzdec8jf2sak7tcj4wal6pm37x3hsrlwtwrk3s26sddscfuh6fnqz62mg9").unwrap(),
        unlock_utxo_value.clone(),
        None,
        None
    );
    let unlock_utxo: TransactionUnspentOutput = TransactionUnspentOutput::new(
        TransactionInput::new(
            TransactionHash::from_hex(
                "4fa70fb8aad8f06c932e1593e71f108438342ddb7e3bb51aabf61a3a5474ff63").unwrap(),
            0,
        ),
        unlock_utxo_info.clone(),
    );
    let unlock_script = PartialPlutusWitness::new(
        PlutusScriptWitness::Ref(ScriptHash::from_hex("ddbebc6263726bcc035934b3b71c1fb144a78a1cbbc57499c45b1e89").unwrap()),
        PlutusData::Integer(BigInt::from_str("0").unwrap()),
    );

    let unlock_input = SingleInputBuilder::from(unlock_utxo.clone())
        .plutus_script_inline_datum(unlock_script, Vec::new())
        .unwrap();


    tx_builder.add_collateral(SingleInputBuilder::from(collateral_1).payment_key().unwrap()).unwrap();
    // tx_builder.add_collateral(SingleInputBuilder::from(collateral_2).payment_key().unwrap()).unwrap();

    tx_builder.add_reference_input(unlock_ref_utxo.clone());

    tx_builder.add_input(SingleInputBuilder::from(user_utxo).payment_key().unwrap()).unwrap();
    tx_builder.add_input(unlock_input.clone()).unwrap();

    tx_builder.set_exunits(
        RedeemerWitnessKey::new(RedeemerTag::Spend, 1),
        POOL_EXECUTION_UNITS,
    );

    tx_builder.add_output(SingleOutputBuilderResult::new(unlock_utxo_output.clone()))
        .unwrap();

    let tx = tx_builder
        .build(ChangeSelectionAlgo::Default, Address::from_bech32("addr1q9u5vlrf4xkxv2qpwngf6cjhtw542ayty80v8dyr49rf5ewvxwdrt70qlcpeeagscasafhffqsxy36t90ldv06wqrk2qld6xc3").as_ref().unwrap())
        .unwrap();

    // println!("Tx bytes: {}", hex::encode(tx.build_unchecked().to_canonical_cbor_bytes()));
    tx.build_checked();

    println!("test");
}

fn to_cml_utxo(exp_tx_out: ExplorerTxOut) -> TransactionUnspentOutput {
    let datum = if let Some(hash) = exp_tx_out.data_hash {
        Some(DatumOption::new_hash(DatumHash::from_hex(hash.as_str()).unwrap()))
    } else if let Some(datum) = exp_tx_out.data {
        Some(DatumOption::new_datum(
            PlutusData::from_bytes(hex::decode(datum).unwrap()).unwrap(),
        ))
    } else {
        None
    };
    let input: TransactionInput = TransactionInput::new(
        TransactionHash::from_hex(exp_tx_out.tx_hash.as_str()).unwrap(),
        exp_tx_out.index,
    );
    let output: TransactionOutput = TransactionOutput::new(
        Address::from_bech32(exp_tx_out.addr.as_str()).unwrap(),
        ExplorerValue::try_into(exp_tx_out.value).unwrap(),
        datum,
        None,
    );
    TransactionUnspentOutput::new(input, output)
}
