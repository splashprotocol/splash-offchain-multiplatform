use bloom_offchain_cardano::orders::spot::SPOT_ORDER_N2T_EX_UNITS;
use cml_chain::builders::redeemer_builder::RedeemerWitnessKey;
use cml_chain::plutus::RedeemerTag;
use cml_chain::{
    address::{Address, BaseAddress, EnterpriseAddress},
    assets::AssetName,
    builders::{
        input_builder::SingleInputBuilder,
        mint_builder::SingleMintBuilder,
        output_builder::{SingleOutputBuilderResult, TransactionOutputBuilder},
        tx_builder::{
            ChangeSelectionAlgo, TransactionBuilder, TransactionBuilderConfigBuilder,
            TransactionUnspentOutput,
        },
        witness_builder::{PartialPlutusWitness, PlutusScriptWitness},
    },
    certs::StakeCredential,
    fees::LinearFee,
    genesis::network_info::NetworkInfo,
    plutus::{ConstrPlutusData, ExUnitPrices, PlutusData, PlutusV2Script},
    transaction::{TransactionInput, TransactionOutput},
    utils::BigInt,
    PolicyId, SubCoin, Value,
};
use cml_core::network::ProtocolMagic;
use cml_crypto::{Bip32PrivateKey, Ed25519KeyHash, PrivateKey, RawBytesEncoding, TransactionHash};
use spectrum_cardano_lib::protocol_params::constant_tx_builder;

fn main() {
    let network_info = NetworkInfo::new(0b0000, ProtocolMagic::from(2_u32));
    let (operator_sk, operator_pkh, operator_addr) = operator_creds("xprv1cr38ar6z5tn4mcjfa2pq49s2cchjpu6nd9qsqu6zxrruqu8kzfduch8repn8ukxdl4qjj4n002rwgf6dhg4ldq23vgsevt6tmnyc657yrsk5t6v3slm33qkh3f0x4xru6ue8w0k0medspw7fqfrmrppm0sdla648", network_info);

    const SPOT_SCRIPT: &str = "5906c301000032323232323232323222232323232533300a32323232323253330103370e90000018991919191919299980b19b8748008c0540044c8c8c94ccc064cdc3a400060300022646464646464a66603e66e1d2000301e0011323232323232323232323232323232323232323232323232323253330395333039533303953330395333039002100314a0200a2940402452808030a50100114a064a66607800229444c8cc004004008894ccc0f800452809919299981e998178188010a511330040040013042002375c60800026eb0c0b4c0d80d4c8c94ccc0e4cdc3a40000022646464a66607800e20022940cdc79bae302c3039038001375c607e002606e00426464646464a66607c66e1d2004303d001132323232325333043533304353330435333043533304333304300e4a0944402452808040a50100714a0200429404004528299982129998212999821299982129998212999821299982119b8f375c6070608007e6eb8c0e0c10000c4cdd79810982001f981098200018a5013375e6026608007e6026608000629404cdd7980c182001f980c18200018a5013370e6eb4c094c1000fcdd6981298200018a5013370e6eb4c024c1000fcdd6980498200018a5013371e6eb8c0ccc1000fcdd7181998200018a5013375e606e608007e606e60800062940cdc39bad3026303f00201030390013044001303c001163020303b01d3375e606460740406064607403866e1cccc060dd59810181c80d9bae30313039038488101010048008cdc49bad300130380370112303f3040304030400013037001302d3035302d30350173375e603660686058606802c6036606860586068034a66606a66e2000c020528899b8700448000cdc42400001666e2520000023370201600266e04dd6980b181781700199b89337040046eb4c098c0b8c004c0b80b4cdc10039bad3015302e3001302e02d23035303630363036303630360013370266e04cdc08048038010009bad3010302b02a3370666e0800cdd6981118150009bad3011302a00130013029028230303031303130313031303130310013370200200666600a6eacc034c098020018dd7180698131803981301298019bab300c30250073330033756601660480140086eb8c02cc090c014c09008cc004dd5980518118049199801000a450048810022232323253330293370e90010008a400026eb4c0b8c09c008c09c004c94ccc0a0cdc3a4004002298103d87a8000132323300100100222533302e00114c103d87a8000132323232533302f3371e014004266e95200033033375000297ae0133006006003375a60600066eb8c0b8008c0c8008c0c0004dd598169813001181300099198008008021129998158008a6103d87a8000132323232533302c3371e010004266e95200033030374c00297ae01330060060033756605a0066eb8c0ac008c0bc008c0b4004dd7180c18101800981000f9181398141814181418140009812800980e8008b1999180080091129998120010a6103d87a80001323253330233370e0069000099ba548000cc09c0092f5c0266600a00a00266e0400d20023028003302600237586002603801601a460466048604800260026034004460426044002603e002602e0022c64646600200200444a66603c0022980103d87a800013232533301d3375e6026603600400c266e952000330210024bd70099802002000981100118100009bac300e3016005301c001301400116301a001301a0023018001301000d375a602c002601c0182660040086eb8c004c0380348c054c058c058c058c058c058c058c05800488c8cc00400400c894ccc05400452809919299980a19b8f00200514a226600800800260320046eb8c05c004c030024dd618009805180118050039180898091809180918091809180918091809000918080008a4c26cac64a66601466e1d200000113232533300f3012002149858dd6980800098040030a99980519b874800800454ccc034c02001852616163008005300100523253330093370e90000008991919191919191919191919191919191919299980f181080109919191924c646600200200a44a6660460022930991980180198138011bae30250013017007301600832533301c3370e9000000899191919299981198130010a4c2c6eb8c090004c090008dd71811000980d0050b180d0048b1bac301f001301f002375c603a002603a0046036002603600460320026032004602e002602e0046eb4c054004c054008dd6980980098098011bad30110013011002375c601e002600e0042c600e002464a66601066e1d2000001132323232533300f3012002149858dd6980800098080011bad300e0013006002163006001230053754002460066ea80055cd2ab9d5573caae7d5d02ba15745";
    let spot_script_inner = PlutusV2Script::new(hex::decode(SPOT_SCRIPT).unwrap());
    let spot_script =
        PlutusScriptWitness::Script(cml_chain::plutus::PlutusScript::PlutusV2(spot_script_inner));

    const MINT_SCRIPT: &str = "5901f001000032323232323232323222253330063253330073370e90001803000899191919299980599b88480000044c94ccc030cdc3a40006016002266ebcc00cc028c044c02800402458ccc8c0040048894ccc044008530103d87a80001323253330103370e0069000099ba548000cc0500092f5c0266600a00a00266e0400d2002301500330130023758600460126004601200c90000a5132323232533300e3370e90010008a400026eb4c04cc030008c030004c94ccc034cdc3a40040022980103d87a8000132323300100100222533301300114c103d87a800013232323253330143371e91101010000213374a90001980c1ba80014bd700998030030019bad3015003375c6026004602e004602a0026eacc048c02c008c02c004c8cc004004008894ccc0400045300103d87a800013232323253330113371e012004266e95200033015374c00297ae0133006006003375660240066eb8c040008c050008c048004c8c8cc004004008894ccc04000452f5bded8c0264646464a66602266e3d221000021003133015337606ea4008dd3000998030030019bab3012003375c6020004602800460240026eacc03cc040c040c040c040c020c004c0200148c03c004dd7180680098028008b1805980618020008a4c26cac4600a6ea80048c00cdd5000ab9a5573aaae7955cfaba05742ae881";
    let mint_script_inner = PlutusV2Script::new(hex::decode(MINT_SCRIPT).unwrap());
    let mint_script =
        PlutusScriptWitness::Script(cml_chain::plutus::PlutusScript::PlutusV2(mint_script_inner));
    let asset = AssetName::new("SN_beacon".as_bytes().to_vec()).unwrap();
    let mint_witness = PartialPlutusWitness::new(mint_script, PlutusData::Integer(BigInt::from(1_u64)));
    let minting_policy =
        SingleMintBuilder::new_single_asset(asset, 1).plutus_script(mint_witness, vec![operator_pkh]);

    //let input_builder = TransactionUnspentOutput
    //

    // Get the input UTxO for this wallet:
    let input: TransactionInput = TransactionInput::new(
        TransactionHash::from_hex("f7bd9877e3e4f0b5c1cef9a9bf780947b128a8beb33a66efe310930f57f2b807")
            .unwrap(),
        0,
    );
    let value = Value::from(10000000000);
    let output: TransactionOutput = TransactionOutput::new(
        Address::from_bech32("addr_test1vzkx6mw5c35slc2t9lzve5g9n0uea7jqma3xsc7nxvuzlhsdsfj4a").unwrap(),
        value.clone(),
        None,
        None,
    );
    let utxo = TransactionUnspentOutput::new(input, output);
    let input_builder_result = SingleInputBuilder::from(utxo).payment_key().unwrap();

    let spot_script_hash = spot_script.hash();
    let contract_address = EnterpriseAddress::new(
        NetworkInfo::mainnet().network_id(),
        StakeCredential::new_script(spot_script_hash),
    )
    .to_address();

    let beacon = PolicyId::from_hex("c3f723bfe736990376a0ccf55498df2b4b69aa33e780d2693703012b").unwrap();

    let output = TransactionOutputBuilder::new()
        .with_address(contract_address)
        .with_data(cml_chain::transaction::DatumOption::new_datum(create_datum(
            &beacon,
            &operator_pkh,
        )))
        .next()
        .unwrap()
        .with_value(Value::from(23913720))
        .build()
        .unwrap();

    //let base = BaseAddress::new(network, payment, stake);
    let mut tx_builder = constant_tx_builder();
    tx_builder.add_input(input_builder_result).unwrap();

    tx_builder.add_output(output).unwrap();
    tx_builder.add_mint(minting_policy).unwrap();
    tx_builder.set_exunits(
        RedeemerWitnessKey::new(RedeemerTag::Mint, 0),
        SPOT_ORDER_N2T_EX_UNITS,
    );
    let tx = tx_builder
        .build(ChangeSelectionAlgo::Default, &operator_addr)
        .unwrap();
    println!("addr: {}", operator_addr.to_bech32(None).unwrap());
}

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

fn create_datum(beacon: &PolicyId, redeemer_pkh: &Ed25519KeyHash) -> PlutusData {
    let beacon_plutus = PlutusData::new_bytes(beacon.to_raw_bytes().to_vec());
    let tradable_input = PlutusData::new_integer(BigInt::from(10000000));
    let cost_per_ex_step = PlutusData::new_integer(BigInt::from(10000));
    let min_marginal_output = PlutusData::new_integer(BigInt::from(1000000));

    let output_token_policy = PlutusData::new_bytes(
        PolicyId::from_hex("4b3459fd18a1dbabe207cd19c9951a9fac9f5c0f9c384e3d97efba26")
            .unwrap()
            .to_raw_bytes()
            .to_vec(),
    );
    let output_token_name = PlutusData::new_bytes(b"testB".to_vec());
    let output = PlutusData::new_constr_plutus_data(ConstrPlutusData::new(
        0,
        vec![output_token_policy, output_token_name],
    ));

    let num = PlutusData::new_integer(BigInt::from(61_u64));
    let denom = PlutusData::new_integer(BigInt::from(100_u64));
    let base_price = PlutusData::ConstrPlutusData(ConstrPlutusData::new(0, vec![num, denom]));

    let num = PlutusData::new_integer(BigInt::from(61_u64));
    let denom = PlutusData::new_integer(BigInt::from(100_u64));
    let fee_per_output = PlutusData::ConstrPlutusData(ConstrPlutusData::new(0, vec![num, denom]));
    let redeemer_reward_pkh = PlutusData::new_bytes(redeemer_pkh.to_raw_bytes().to_vec());
    let permitted_executors = PlutusData::new_list(vec![]);
    PlutusData::ConstrPlutusData(ConstrPlutusData::new(
        0,
        vec![
            beacon_plutus,
            tradable_input,
            cost_per_ex_step,
            min_marginal_output,
            output,
            base_price,
            fee_per_output,
            redeemer_reward_pkh,
            permitted_executors,
        ],
    ))
}
