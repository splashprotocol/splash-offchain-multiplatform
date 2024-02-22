use std::sync::{Arc, Once};

use async_stream::stream;
use cml_chain::address::{Address, EnterpriseAddress};
use cml_chain::assets::{AssetName, MultiAsset};
use cml_chain::auxdata::{AuxiliaryData, ConwayFormatAuxData};
use cml_chain::builders::input_builder::InputBuilderResult;
use cml_chain::builders::tx_builder::TransactionUnspentOutput;
use cml_chain::builders::witness_builder::RequiredWitnessSet;
use cml_chain::certs::{Certificate, StakeCredential};
use cml_chain::genesis::network_info::NetworkInfo;
use cml_chain::plutus::{ConstrPlutusData, PlutusData, PlutusV2Script};
use cml_chain::transaction::cbor_encodings::{
    ConwayFormatTxOutEncoding, TransactionBodyEncoding, TransactionEncoding, TransactionInputEncoding,
};
use cml_chain::transaction::{
    ConwayFormatTxOut, DatumOption, ScriptRef, Transaction, TransactionBody, TransactionInput,
    TransactionOutput, TransactionWitnessSet,
};
use cml_chain::utils::BigInt;
use cml_chain::{NetworkId, OrderedHashMap, PolicyId, Script, Value};
use cml_core::network::ProtocolMagic;
use cml_crypto::{Ed25519KeyHash, RawBytesEncoding, TransactionHash};
use cml_multi_era::allegra::AllegraCertificate;
use cml_multi_era::babbage::cbor_encodings::{
    BabbageFormatAuxDataEncoding, BabbageFormatTxOutEncoding, BabbageTransactionBodyEncoding,
    BabbageTransactionEncoding,
};
use cml_multi_era::babbage::{
    BabbageAuxiliaryData, BabbageFormatAuxData, BabbageFormatTxOut, BabbageScript, BabbageTransaction,
    BabbageTransactionBody, BabbageTransactionOutput, BabbageTransactionWitnessSet,
};
use either::Either;
use futures::channel::mpsc;
use futures::stream::select_all;
use futures::{Stream, StreamExt};
use log::{info, trace};
use rand::Rng;
use spectrum_cardano_lib::collateral::Collateral;
use spectrum_offchain::network::Network;
use spectrum_offchain_cardano::constants::{
    DEPOSIT_SCRIPT, FEE_SWITCH_POOL_SCRIPT, FEE_SWITCH_POOL_SCRIPT_BIDIRECTIONAL_FEE_SCRIPT, POOL_V1_SCRIPT,
    POOL_V2_SCRIPT, REDEEM_SCRIPT, SWAP_SCRIPT,
};
use tokio::sync::Mutex;
use tracing_subscriber::fmt::Subscriber;

use bloom_offchain::execution_engine::bundled::Bundled;
use bloom_offchain::execution_engine::execution_part_stream;
use bloom_offchain::execution_engine::liquidity_book::{ExecutionCap, TLB};
use bloom_offchain::execution_engine::multi_pair::MultiPair;
use bloom_offchain::execution_engine::storage::cache::InMemoryStateIndexCache;
use bloom_offchain::execution_engine::storage::InMemoryStateIndex;
use bloom_offchain_cardano::event_sink::entity_index::InMemoryEntityIndex;
use bloom_offchain_cardano::event_sink::handler::PairUpdateHandler;
use bloom_offchain_cardano::event_sink::CardanoEntity;
use bloom_offchain_cardano::execution_engine::interpreter::CardanoRecipeInterpreter;
use bloom_offchain_cardano::orders::AnyOrder;
use bloom_offchain_cardano::pools::AnyPool;
use cardano_chain_sync::data::LedgerTxEvent;
use spectrum_cardano_lib::output::FinalizedTxOut;
use spectrum_cardano_lib::OutputRef;
use spectrum_offchain::data::unique_entity::{EitherMod, StateUpdate};
use spectrum_offchain::data::Baked;
use spectrum_offchain::event_sink::event_handler::EventHandler;
use spectrum_offchain::event_sink::process_events;
use spectrum_offchain::partitioning::Partitioned;
use spectrum_offchain::streaming::boxed;
use spectrum_offchain_cardano::creds::operator_creds;
use spectrum_offchain_cardano::data::pair::PairId;
use spectrum_offchain_cardano::data::ref_scripts::ReferenceOutputs;
use spectrum_offchain_cardano::prover::operator::OperatorProver;
use spectrum_offchain_cardano::tx_submission::TxRejected;

use bloom_cardano_agent::config::AppConfig;
use bloom_cardano_agent::context::ExecutionContext;

const EXECUTION_CAP: ExecutionCap = ExecutionCap {
    soft: 6000000000,
    hard: 10000000000,
};

#[tokio::test]
// #[ignore]
async fn integration_test() {
    let subscriber = Subscriber::new();
    tracing::subscriber::set_global_default(subscriber).expect("setting tracing default failed");
    let raw_config = std::fs::read_to_string("tests/config.json").expect("Cannot load configuration file");
    let config: AppConfig = serde_json::from_str(&raw_config).expect("Invalid configuration file");

    log4rs::init_file("tests/log4rs.yaml", Default::default()).unwrap();

    info!("Starting Off-Chain Agent ..");

    let network_info = NetworkInfo::new(0b0000, ProtocolMagic::from(1_u32));

    let ref_scripts = gen_reference_outputs();

    let (tx_sender, mut tx_receiver) = tokio::sync::mpsc::channel(10);
    let tx_sender = TxSubmissionSender(tx_sender);

    // prepare upstreams
    let signal_tip_reached = Once::new();

    let n_id = network_info.network_id() as u64;
    let (pool_tx, token_policy, token_name) = gen_pool_babbage_tx(NetworkId::new(n_id));
    let spot_order_tx_0 = gen_spot_order_babbage_tx(NetworkId::new(n_id), token_policy, token_name);
    let ledger_stream = Box::pin(
        futures::stream::iter(vec![
            LedgerTxEvent::TxApplied {
                tx: pool_tx,
                slot: 49635947,
            },
            LedgerTxEvent::TxApplied {
                tx: spot_order_tx_0,
                slot: 49636000,
            },
        ])
        .chain(stream! {
            let mut slot = 49636100;
            while let Some(tx) = tx_receiver.recv().await {
                //let spot_order_tx_1 = gen_spot_order_babbage_tx(NetworkId::new(n_id), token_policy, token_name);
                yield  LedgerTxEvent::TxApplied {
                    tx: to_babbage_transaction(tx),
                    slot,
                };
                slot += 100;
            }
        }),
    );

    signal_tip_reached.call_once(|| {
        trace!(target: "chain_sync", "Tip reached, waiting for new blocks ..");
    });

    let (operator_sk, operator_pkh, _operator_addr) = operator_creds(config.batcher_private_key, 1_u64);

    let collateral = gen_collateral();

    let (pair_upd_snd_p1, pair_upd_recv_p1) =
        mpsc::channel::<(PairId, EitherMod<StateUpdate<CardanoEntity>>)>(128);
    let (pair_upd_snd_p2, pair_upd_recv_p2) =
        mpsc::channel::<(PairId, EitherMod<StateUpdate<CardanoEntity>>)>(128);
    let (pair_upd_snd_p3, pair_upd_recv_p3) =
        mpsc::channel::<(PairId, EitherMod<StateUpdate<CardanoEntity>>)>(128);
    let (pair_upd_snd_p4, pair_upd_recv_p4) =
        mpsc::channel::<(PairId, EitherMod<StateUpdate<CardanoEntity>>)>(128);

    let partitioned_pair_upd_snd =
        Partitioned::new([pair_upd_snd_p1, pair_upd_snd_p2, pair_upd_snd_p3, pair_upd_snd_p4]);
    let index = Arc::new(Mutex::new(InMemoryEntityIndex::new(
        config.cardano_finalization_delay,
    )));
    let upd_handler = PairUpdateHandler::new(partitioned_pair_upd_snd, index, config.executor_cred);

    let handlers_ledger: Vec<Box<dyn EventHandler<LedgerTxEvent<BabbageTransaction>>>> =
        vec![Box::new(upd_handler.clone())];

    let prover = OperatorProver::new(&operator_sk);
    let interpreter = CardanoRecipeInterpreter;
    let context = ExecutionContext {
        time: 0.into(),
        refs: ref_scripts,
        execution_cap: EXECUTION_CAP,
        reward_addr: config.reward_address,
        collateral,
    };
    let multi_book = MultiPair::new::<TLB<AnyOrder, AnyPool>>(context.clone());
    let state_index = InMemoryStateIndex::new();
    let state_cache = InMemoryStateIndexCache::new();

    let execution_stream_p1 = execution_part_stream(
        state_index.clone(),
        state_cache.clone(),
        multi_book.clone(),
        context.clone(),
        interpreter,
        prover,
        unwrap_updates(pair_upd_recv_p1),
        tx_sender.clone(),
    );
    let execution_stream_p2 = execution_part_stream(
        state_index.clone(),
        state_cache.clone(),
        multi_book.clone(),
        context.clone(),
        interpreter,
        prover,
        unwrap_updates(pair_upd_recv_p2),
        tx_sender.clone(),
    );
    let execution_stream_p3 = execution_part_stream(
        state_index.clone(),
        state_cache.clone(),
        multi_book.clone(),
        context.clone(),
        interpreter,
        prover,
        unwrap_updates(pair_upd_recv_p3),
        tx_sender.clone(),
    );
    let execution_stream_p4 = execution_part_stream(
        state_index,
        state_cache,
        multi_book,
        context,
        interpreter,
        prover,
        unwrap_updates(pair_upd_recv_p4),
        tx_sender,
    );

    let process_ledger_events_stream = process_events(ledger_stream, handlers_ledger);

    let mut app = select_all(vec![
        boxed(process_ledger_events_stream),
        boxed(execution_stream_p1),
        boxed(execution_stream_p2),
        boxed(execution_stream_p3),
        boxed(execution_stream_p4),
    ]);

    loop {
        app.select_next_some().await;
    }
}

fn unwrap_updates(
    upstream: impl Stream<Item = (PairId, EitherMod<StateUpdate<CardanoEntity>>)>,
) -> impl Stream<
    Item = (
        PairId,
        EitherMod<
            StateUpdate<
                Bundled<Either<Baked<AnyOrder, OutputRef>, Baked<AnyPool, OutputRef>>, FinalizedTxOut>,
            >,
        >,
    ),
> {
    upstream.map(|(p, m)| (p, m.map(|s| s.map(|CardanoEntity(e)| e))))
}

#[derive(Clone)]
struct TxSubmissionSender(tokio::sync::mpsc::Sender<Transaction>);

#[async_trait::async_trait]
impl Network<Transaction, TxRejected> for TxSubmissionSender {
    async fn submit_tx(&mut self, tx: Transaction) -> Result<(), TxRejected> {
        self.0.send(tx).await.unwrap();
        Ok(())
    }
}

fn gen_pool_babbage_tx(network_id: NetworkId) -> (BabbageTransaction, PolicyId, &'static str) {
    let (body, token_policy, token_name) = gen_pool_transaction_body(network_id);
    let tx = BabbageTransaction::new(body, BabbageTransactionWitnessSet::new(), true, None);
    (tx, token_policy, token_name)
}

fn gen_pool_transaction_body(network_id: NetworkId) -> (BabbageTransactionBody, PolicyId, &'static str) {
    let (output, token_policy, token_name) = gen_pool_transaction_output(0);
    let body = BabbageTransactionBody {
        inputs: vec![gen_transaction_input(0), gen_transaction_input(1)],
        outputs: vec![output],
        fee: 281564,
        ttl: None,
        certs: None,
        withdrawals: None,
        update: None,
        auxiliary_data_hash: None,
        validity_interval_start: Some(51438385),
        mint: None,
        script_data_hash: None,
        collateral_inputs: None,
        required_signers: None,
        network_id: Some(network_id),
        collateral_return: None,
        total_collateral: Some(3607615),
        reference_inputs: None,
        encodings: Some(BabbageTransactionBodyEncoding::default()),
    };
    (body, token_policy, token_name)
}

fn gen_transaction_input(index: u64) -> TransactionInput {
    let mut rng = rand::thread_rng();
    let mut bytes = [0u8; 32];
    rng.fill(&mut bytes[..]);
    let transaction_id = TransactionHash::from(bytes);
    let encodings = Some(TransactionInputEncoding {
        len_encoding: cml_chain::LenEncoding::Canonical,
        transaction_id_encoding: cml_crypto::StringEncoding::Definite(cbor_event::Sz::One),
        index_encoding: Some(cbor_event::Sz::Inline),
    });
    TransactionInput {
        transaction_id,
        index,
        encodings,
    }
}

fn gen_policy_id() -> PolicyId {
    let mut rng = rand::thread_rng();
    let mut bytes = [0u8; 28];
    rng.fill(&mut bytes[..]);
    PolicyId::from(bytes)
}

fn gen_pool_transaction_output(index: u64) -> (BabbageTransactionOutput, PolicyId, &'static str) {
    let network = NetworkInfo::new(0b0000, ProtocolMagic::from(1_u32)).network_id();
    let script_plutus_v2 = PlutusV2Script::new(hex::decode(POOL_V2_SCRIPT).unwrap());
    let script = Script::new_plutus_v2(script_plutus_v2.clone());
    let address =
        EnterpriseAddress::new(network, StakeCredential::new_script(script.clone().hash())).to_address();
    info!(
        "pool_v2_addr: {}, {:?}",
        address.to_bech32(None).unwrap(),
        address
    );

    let pool_y_token_policy = gen_policy_id();
    let pool_y_token_name = "token Y";
    let pool_lq_token_policy = gen_policy_id();
    let pool_lq_token_name = "token LQ";

    let mut bundle = OrderedHashMap::new();
    bundle.insert(
        pool_y_token_policy,
        OrderedHashMap::from_iter(vec![(
            AssetName::try_from(pool_y_token_name.as_bytes().to_vec()).unwrap(),
            100_000_000,
        )]),
    );
    bundle.insert(
        pool_lq_token_policy,
        OrderedHashMap::from_iter(vec![(
            AssetName::try_from(pool_lq_token_name.as_bytes().to_vec()).unwrap(),
            10_000_000,
        )]),
    );
    let amount = Value::new(100_000_000, MultiAsset::from(bundle));

    let datum_option = Some(DatumOption::new_datum(gen_pool_datum(
        pool_y_token_policy,
        pool_y_token_name,
        pool_lq_token_policy,
        pool_lq_token_name,
    )));

    // Don't need to set script_reference since the code uses reference inputs for the pool
    let tx = BabbageFormatTxOut {
        address,
        amount,
        datum_option,
        script_reference: None, //Some(BabbageScript::new_plutus_v2(script_plutus_v2)),
        encodings: None,
    };
    (
        BabbageTransactionOutput::BabbageFormatTxOut(tx),
        pool_y_token_policy,
        pool_y_token_name,
    )
}

// Datum for an ADA-token pool
fn gen_pool_datum(
    pool_y_token_policy: PolicyId,
    pool_y_token_name: &str,
    pool_lq_token_policy: PolicyId,
    pool_lq_token_name: &str,
) -> PlutusData {
    // Pool NFT
    let nft_token_policy = PlutusData::new_bytes(gen_policy_id().to_raw_bytes().to_vec());
    let nft_token_name = PlutusData::new_bytes(b"pool_NFT".to_vec());
    let nft_output =
        PlutusData::new_constr_plutus_data(ConstrPlutusData::new(0, vec![nft_token_policy, nft_token_name]));

    // ADA (pool X)
    let ada_bytes = PlutusData::new_bytes(vec![]);
    let ada_name_bytes = PlutusData::new_bytes(vec![]);

    let pool_x_output =
        PlutusData::new_constr_plutus_data(ConstrPlutusData::new(0, vec![ada_bytes, ada_name_bytes]));

    // Pool Y
    let pool_y_token_policy = PlutusData::new_bytes(pool_y_token_policy.to_raw_bytes().to_vec());
    let pool_y_token_name = PlutusData::new_bytes(pool_y_token_name.as_bytes().to_vec());
    let pool_y_output = PlutusData::new_constr_plutus_data(ConstrPlutusData::new(
        0,
        vec![pool_y_token_policy, pool_y_token_name],
    ));

    // Pool LQ
    let pool_lq_token_policy = PlutusData::new_bytes(pool_lq_token_policy.to_raw_bytes().to_vec());
    let pool_lq_token_name = PlutusData::new_bytes(pool_lq_token_name.as_bytes().to_vec());
    let pool_lq_output = PlutusData::new_constr_plutus_data(ConstrPlutusData::new(
        0,
        vec![pool_lq_token_policy, pool_lq_token_name],
    ));

    // Fee
    let fee_output = PlutusData::new_integer(BigInt::from(9995));

    // Dummy value
    let dummy_output = PlutusData::new_integer(BigInt::from(1));

    // LQ lower bound
    let lq_lower_bound_output = PlutusData::new_integer(BigInt::from(10000000000_u64));

    PlutusData::ConstrPlutusData(ConstrPlutusData::new(
        0,
        vec![
            nft_output,
            pool_x_output,
            pool_y_output,
            pool_lq_output,
            fee_output,
            dummy_output,
            lq_lower_bound_output,
        ],
    ))
}

fn gen_spot_order_babbage_tx(
    network_id: NetworkId,
    output_token_policy: PolicyId,
    output_token_name: &str,
) -> BabbageTransaction {
    let witness_set = BabbageTransactionWitnessSet::new();
    BabbageTransaction::new(
        gen_spot_order_transaction_body(network_id, output_token_policy, output_token_name),
        witness_set,
        true,
        None,
    )
}

fn gen_spot_order_transaction_body(
    network_id: NetworkId,
    output_token_policy: PolicyId,
    output_token_name: &str,
) -> BabbageTransactionBody {
    BabbageTransactionBody {
        inputs: vec![gen_transaction_input(0), gen_transaction_input(1)],
        outputs: vec![gen_spot_order_transaction_output(
            0,
            output_token_policy,
            output_token_name,
        )],
        fee: 281564,
        ttl: None,
        certs: None,
        withdrawals: None,
        update: None,
        auxiliary_data_hash: None,
        validity_interval_start: Some(51438385),
        mint: None,
        script_data_hash: None,
        collateral_inputs: None,
        required_signers: None,
        network_id: Some(network_id),
        collateral_return: None,
        total_collateral: Some(3607615),
        reference_inputs: None,
        encodings: Some(BabbageTransactionBodyEncoding::default()),
    }
}

fn gen_spot_order_transaction_output(
    index: u64,
    output_token_policy: PolicyId,
    output_token_name: &str,
) -> BabbageTransactionOutput {
    let network_info = NetworkInfo::new(0b0000, ProtocolMagic::from(1_u32));
    let network = network_info.network_id();
    let (operator_sk, operator_pkh, operator_addr) = operator_creds("xprv1cr38ar6z5tn4mcjfa2pq49s2cchjpu6nd9qsqu6zxrruqu8kzfduch8repn8ukxdl4qjj4n002rwgf6dhg4ldq23vgsevt6tmnyc657yrsk5t6v3slm33qkh3f0x4xru6ue8w0k0medspw7fqfrmrppm0sdla648", 1_u64);
    const SPOT_SCRIPT: &str = "5906c301000032323232323232323222232323232533300a32323232323253330103370e90000018991919191919299980b19b8748008c0540044c8c8c94ccc064cdc3a400060300022646464646464a66603e66e1d2000301e0011323232323232323232323232323232323232323232323232323253330395333039533303953330395333039002100314a0200a2940402452808030a50100114a064a66607800229444c8cc004004008894ccc0f800452809919299981e998178188010a511330040040013042002375c60800026eb0c0b4c0d80d4c8c94ccc0e4cdc3a40000022646464a66607800e20022940cdc79bae302c3039038001375c607e002606e00426464646464a66607c66e1d2004303d001132323232325333043533304353330435333043533304333304300e4a0944402452808040a50100714a0200429404004528299982129998212999821299982129998212999821299982119b8f375c6070608007e6eb8c0e0c10000c4cdd79810982001f981098200018a5013375e6026608007e6026608000629404cdd7980c182001f980c18200018a5013370e6eb4c094c1000fcdd6981298200018a5013370e6eb4c024c1000fcdd6980498200018a5013371e6eb8c0ccc1000fcdd7181998200018a5013375e606e608007e606e60800062940cdc39bad3026303f00201030390013044001303c001163020303b01d3375e606460740406064607403866e1cccc060dd59810181c80d9bae30313039038488101010048008cdc49bad300130380370112303f3040304030400013037001302d3035302d30350173375e603660686058606802c6036606860586068034a66606a66e2000c020528899b8700448000cdc42400001666e2520000023370201600266e04dd6980b181781700199b89337040046eb4c098c0b8c004c0b80b4cdc10039bad3015302e3001302e02d23035303630363036303630360013370266e04cdc08048038010009bad3010302b02a3370666e0800cdd6981118150009bad3011302a00130013029028230303031303130313031303130310013370200200666600a6eacc034c098020018dd7180698131803981301298019bab300c30250073330033756601660480140086eb8c02cc090c014c09008cc004dd5980518118049199801000a450048810022232323253330293370e90010008a400026eb4c0b8c09c008c09c004c94ccc0a0cdc3a4004002298103d87a8000132323300100100222533302e00114c103d87a8000132323232533302f3371e014004266e95200033033375000297ae0133006006003375a60600066eb8c0b8008c0c8008c0c0004dd598169813001181300099198008008021129998158008a6103d87a8000132323232533302c3371e010004266e95200033030374c00297ae01330060060033756605a0066eb8c0ac008c0bc008c0b4004dd7180c18101800981000f9181398141814181418140009812800980e8008b1999180080091129998120010a6103d87a80001323253330233370e0069000099ba548000cc09c0092f5c0266600a00a00266e0400d20023028003302600237586002603801601a460466048604800260026034004460426044002603e002602e0022c64646600200200444a66603c0022980103d87a800013232533301d3375e6026603600400c266e952000330210024bd70099802002000981100118100009bac300e3016005301c001301400116301a001301a0023018001301000d375a602c002601c0182660040086eb8c004c0380348c054c058c058c058c058c058c058c05800488c8cc00400400c894ccc05400452809919299980a19b8f00200514a226600800800260320046eb8c05c004c030024dd618009805180118050039180898091809180918091809180918091809000918080008a4c26cac64a66601466e1d200000113232533300f3012002149858dd6980800098040030a99980519b874800800454ccc034c02001852616163008005300100523253330093370e90000008991919191919191919191919191919191919299980f181080109919191924c646600200200a44a6660460022930991980180198138011bae30250013017007301600832533301c3370e9000000899191919299981198130010a4c2c6eb8c090004c090008dd71811000980d0050b180d0048b1bac301f001301f002375c603a002603a0046036002603600460320026032004602e002602e0046eb4c054004c054008dd6980980098098011bad30110013011002375c601e002600e0042c600e002464a66601066e1d2000001132323232533300f3012002149858dd6980800098080011bad300e0013006002163006001230053754002460066ea80055cd2ab9d5573caae7d5d02ba15745";
    let spot_script_inner = PlutusV2Script::new(hex::decode(SPOT_SCRIPT).unwrap());

    let script = Script::new_plutus_v2(spot_script_inner.clone());
    let address =
        EnterpriseAddress::new(network, StakeCredential::new_script(script.clone().hash())).to_address();
    info!(target:"offchain", "spot order address: {:?}", address);

    let beacon_token_policy = gen_policy_id();
    let beacon_token_name = "beacon";

    let mut bundle = OrderedHashMap::new();
    bundle.insert(
        beacon_token_policy,
        OrderedHashMap::from_iter(vec![(
            AssetName::try_from(beacon_token_name.as_bytes().to_vec()).unwrap(),
            1,
        )]),
    );
    bundle.insert(
        output_token_policy,
        OrderedHashMap::from_iter(vec![(
            AssetName::try_from(output_token_name.as_bytes().to_vec()).unwrap(),
            1,
        )]),
    );
    let amount = Value::new(50_000_000, MultiAsset::from(bundle));

    let datum_option = Some(DatumOption::new_datum(gen_spot_order_datum(
        &beacon_token_policy,
        &operator_pkh,
        output_token_policy,
        output_token_name,
    )));

    // TODO: use input references for spot, allowing us to set script_reference to None. Saving TX costs and min ADA value.
    let tx = BabbageFormatTxOut {
        address,
        amount,
        datum_option,
        script_reference: Some(BabbageScript::new_plutus_v2(spot_script_inner)),
        encodings: None,
    };
    BabbageTransactionOutput::BabbageFormatTxOut(tx)
}

fn gen_spot_order_datum(
    beacon: &PolicyId,
    redeemer_pkh: &Ed25519KeyHash,
    output_token_policy: PolicyId,
    output_token_name: &str,
) -> PlutusData {
    let beacon_pd = PlutusData::new_bytes(beacon.to_raw_bytes().to_vec());
    let tradable_input = 37_000_000_u64;
    let fee = 1_000_000;
    let cost_per_ex_step = 300_000;
    let tradable_input_pd = PlutusData::new_integer(BigInt::from(tradable_input));
    let cost_per_ex_step_pd = PlutusData::new_integer(BigInt::from(cost_per_ex_step));
    let max_ex_steps = 5;
    let base_price_num = 50;
    let base_price_denom = 100;
    let min_final_output = tradable_input * base_price_num / base_price_denom;
    let min_marginal_output = min_final_output / max_ex_steps;
    let min_marginal_output_pd = PlutusData::new_integer(BigInt::from(min_marginal_output));

    let output_token_policy = PlutusData::new_bytes(output_token_policy.to_raw_bytes().to_vec());
    let output_token_name = PlutusData::new_bytes(output_token_name.as_bytes().to_vec());
    let output = PlutusData::new_constr_plutus_data(ConstrPlutusData::new(
        0,
        vec![output_token_policy, output_token_name],
    ));

    let num_pd = PlutusData::new_integer(BigInt::from(base_price_num));
    let denom_pd = PlutusData::new_integer(BigInt::from(base_price_denom));
    let base_price_pd = PlutusData::ConstrPlutusData(ConstrPlutusData::new(0, vec![num_pd, denom_pd]));

    let fee_per_output = PlutusData::new_integer(BigInt::from(fee));
    let redeemer_reward_pkh = PlutusData::new_bytes(redeemer_pkh.to_raw_bytes().to_vec());
    let permitted_executors = PlutusData::new_list(vec![]);
    PlutusData::ConstrPlutusData(ConstrPlutusData::new(
        0,
        vec![
            beacon_pd,
            tradable_input_pd,
            cost_per_ex_step_pd,
            min_marginal_output_pd,
            output,
            base_price_pd,
            fee_per_output,
            redeemer_reward_pkh,
            permitted_executors,
        ],
    ))
}

fn to_babbage_witness_set(ws: TransactionWitnessSet) -> BabbageTransactionWitnessSet {
    BabbageTransactionWitnessSet {
        vkeywitnesses: ws.vkeywitnesses,
        native_scripts: ws.native_scripts,
        bootstrap_witnesses: ws.bootstrap_witnesses,
        plutus_v1_scripts: ws.plutus_v1_scripts,
        plutus_datums: ws.plutus_datums,
        redeemers: ws.redeemers,
        plutus_v2_scripts: ws.plutus_v2_scripts,
        encodings: None,
    }
}

fn to_babbage_transaction_output(output: TransactionOutput) -> BabbageTransactionOutput {
    match output {
        TransactionOutput::AlonzoFormatTxOut(tx_out) => BabbageTransactionOutput::AlonzoFormatTxOut(tx_out),
        TransactionOutput::ConwayFormatTxOut(ConwayFormatTxOut {
            address,
            amount,
            datum_option,
            script_reference,
            encodings,
        }) => BabbageTransactionOutput::BabbageFormatTxOut(BabbageFormatTxOut {
            address,
            amount,
            datum_option,
            script_reference: script_reference.map(to_babbage_script),
            encodings: encodings.map(to_babbage_encoding),
        }),
    }
}

fn to_babbage_script(script: Script) -> BabbageScript {
    match script {
        Script::Native {
            script,
            len_encoding,
            tag_encoding,
        } => BabbageScript::Native {
            script,
            len_encoding,
            tag_encoding,
        },
        Script::PlutusV1 {
            script,
            len_encoding,
            tag_encoding,
        } => BabbageScript::PlutusV1 {
            script,
            len_encoding,
            tag_encoding,
        },
        Script::PlutusV2 {
            script,
            len_encoding,
            tag_encoding,
        } => BabbageScript::PlutusV2 {
            script,
            len_encoding,
            tag_encoding,
        },
        Script::PlutusV3 {
            script,
            len_encoding,
            tag_encoding,
        } => unreachable!(),
    }
}

fn to_babbage_encoding(enc: ConwayFormatTxOutEncoding) -> BabbageFormatTxOutEncoding {
    let ConwayFormatTxOutEncoding {
        len_encoding,
        orig_deser_order,
        address_key_encoding,
        amount_key_encoding,
        datum_option_key_encoding,
        script_reference_tag_encoding,
        script_reference_bytes_encoding,
        script_reference_key_encoding,
    } = enc;

    BabbageFormatTxOutEncoding {
        len_encoding,
        orig_deser_order,
        address_key_encoding,
        amount_key_encoding,
        datum_option_key_encoding,
        script_reference_tag_encoding,
        script_reference_bytes_encoding,
        script_reference_key_encoding,
    }
}

fn to_babbage_transaction_body(body: TransactionBody) -> BabbageTransactionBody {
    let outputs = body
        .outputs
        .into_iter()
        .map(to_babbage_transaction_output)
        .collect();
    let certs = body.certs.map(|c| c.into_iter().map(to_allegra_cert).collect());
    BabbageTransactionBody {
        inputs: body.inputs,
        outputs,
        fee: body.fee,
        ttl: body.ttl,
        certs,
        withdrawals: body.withdrawals,
        update: None,
        auxiliary_data_hash: body.auxiliary_data_hash,
        validity_interval_start: body.validity_interval_start,
        mint: body.mint,
        script_data_hash: body.script_data_hash,
        collateral_inputs: body.collateral_inputs,
        required_signers: body.required_signers,
        network_id: body.network_id,
        collateral_return: body.collateral_return.map(to_babbage_transaction_output),
        total_collateral: body.total_collateral,
        reference_inputs: body.reference_inputs,
        encodings: body.encodings.map(to_babbage_transaction_body_encoding),
    }
}

fn to_allegra_cert(cert: Certificate) -> AllegraCertificate {
    match cert {
        Certificate::StakeRegistration(s) => AllegraCertificate::StakeRegistration(s),
        Certificate::StakeDeregistration(s) => AllegraCertificate::StakeDeregistration(s),
        Certificate::StakeDelegation(s) => AllegraCertificate::StakeDelegation(s),
        Certificate::PoolRegistration(p) => AllegraCertificate::PoolRegistration(p),
        Certificate::PoolRetirement(p) => AllegraCertificate::PoolRetirement(p),
        _ => panic!("Variants not supported"),
    }
}

fn to_babbage_transaction_body_encoding(enc: TransactionBodyEncoding) -> BabbageTransactionBodyEncoding {
    BabbageTransactionBodyEncoding {
        len_encoding: enc.len_encoding,
        orig_deser_order: enc.orig_deser_order,
        inputs_encoding: enc.inputs_encoding,
        inputs_key_encoding: enc.inputs_key_encoding,
        outputs_encoding: enc.outputs_encoding,
        outputs_key_encoding: enc.outputs_key_encoding,
        fee_encoding: enc.fee_encoding,
        fee_key_encoding: enc.fee_key_encoding,
        ttl_encoding: enc.ttl_encoding,
        ttl_key_encoding: enc.ttl_key_encoding,
        certs_encoding: enc.certs_encoding,
        certs_key_encoding: enc.certs_key_encoding,
        withdrawals_encoding: enc.withdrawals_encoding,
        withdrawals_value_encodings: enc.withdrawals_value_encodings,
        withdrawals_key_encoding: enc.withdrawals_key_encoding,
        update_key_encoding: None,
        auxiliary_data_hash_encoding: enc.auxiliary_data_hash_encoding,
        auxiliary_data_hash_key_encoding: enc.auxiliary_data_hash_key_encoding,
        validity_interval_start_encoding: enc.validity_interval_start_encoding,
        validity_interval_start_key_encoding: enc.validity_interval_start_key_encoding,
        mint_encoding: enc.mint_encoding,
        mint_key_encodings: enc.mint_key_encodings,
        mint_value_encodings: enc.mint_value_encodings,
        mint_key_encoding: enc.mint_key_encoding,
        script_data_hash_encoding: enc.script_data_hash_encoding,
        script_data_hash_key_encoding: enc.script_data_hash_key_encoding,
        collateral_inputs_encoding: enc.collateral_inputs_encoding,
        collateral_inputs_key_encoding: enc.collateral_inputs_key_encoding,
        required_signers_encoding: enc.required_signers_encoding,
        required_signers_elem_encodings: enc.required_signers_elem_encodings,
        required_signers_key_encoding: enc.required_signers_key_encoding,
        network_id_key_encoding: enc.network_id_key_encoding,
        collateral_return_key_encoding: enc.collateral_return_key_encoding,
        total_collateral_encoding: enc.total_collateral_encoding,
        total_collateral_key_encoding: enc.total_collateral_key_encoding,
        reference_inputs_encoding: enc.reference_inputs_encoding,
        reference_inputs_key_encoding: enc.reference_inputs_key_encoding,
    }
}

fn to_babbage_auxiliary_data(d: AuxiliaryData) -> BabbageAuxiliaryData {
    match d {
        AuxiliaryData::Shelley(s) => BabbageAuxiliaryData::Shelley(s),
        AuxiliaryData::ShelleyMA(s) => BabbageAuxiliaryData::ShelleyMA(s),
        AuxiliaryData::Conway(ConwayFormatAuxData {
            metadata,
            native_scripts,
            plutus_v1_scripts,
            plutus_v2_scripts,
            plutus_v3_scripts,
            encodings,
        }) => {
            let encodings = encodings.map(|c| BabbageFormatAuxDataEncoding {
                len_encoding: c.len_encoding,
                tag_encoding: c.tag_encoding,
                orig_deser_order: c.orig_deser_order,
                metadata_key_encoding: c.metadata_key_encoding,
                native_scripts_encoding: c.native_scripts_encoding,
                native_scripts_key_encoding: c.native_scripts_key_encoding,
                plutus_v1_scripts_encoding: c.plutus_v1_scripts_encoding,
                plutus_v1_scripts_key_encoding: c.plutus_v1_scripts_key_encoding,
                plutus_v2_scripts_encoding: c.plutus_v2_scripts_encoding,
                plutus_v2_scripts_key_encoding: c.plutus_v2_scripts_key_encoding,
            });

            BabbageAuxiliaryData::Babbage(BabbageFormatAuxData {
                metadata,
                native_scripts,
                plutus_v1_scripts,
                plutus_v2_scripts,
                encodings,
            })
        }
    }
}

fn to_babbage_transaction_encoding(enc: TransactionEncoding) -> BabbageTransactionEncoding {
    BabbageTransactionEncoding {
        len_encoding: enc.len_encoding,
    }
}

fn to_babbage_transaction(tx: Transaction) -> BabbageTransaction {
    BabbageTransaction {
        body: to_babbage_transaction_body(tx.body),
        witness_set: to_babbage_witness_set(tx.witness_set),
        is_valid: tx.is_valid,
        auxiliary_data: tx.auxiliary_data.map(to_babbage_auxiliary_data),
        encodings: tx.encodings.map(to_babbage_transaction_encoding),
    }
}

fn gen_tx_unspent_output(hex_encoded_script: &str) -> TransactionUnspentOutput {
    let input = gen_transaction_input(0);
    let script_ref: ScriptRef =
        Script::new_plutus_v2(PlutusV2Script::new(hex::decode(hex_encoded_script).unwrap()));
    let output = TransactionOutput::new(
        compute_enterprise_addr(script_ref.clone()),
        Value::new(50_000_000, MultiAsset::new()),
        None,
        Some(script_ref),
    );

    TransactionUnspentOutput { input, output }
}

fn compute_enterprise_addr(script_ref: ScriptRef) -> Address {
    EnterpriseAddress::new(
        NetworkInfo::new(0, ProtocolMagic::from(1)).network_id(),
        StakeCredential::new_script(script_ref.hash()),
    )
    .to_address()
}

fn gen_reference_outputs() -> ReferenceOutputs {
    let pool_v1 = gen_tx_unspent_output(POOL_V1_SCRIPT);
    let pool_v2 = gen_tx_unspent_output(POOL_V2_SCRIPT);
    let fee_switch_pool = gen_tx_unspent_output(FEE_SWITCH_POOL_SCRIPT);
    let fee_switch_pool_bidirectional_fee =
        gen_tx_unspent_output(FEE_SWITCH_POOL_SCRIPT_BIDIRECTIONAL_FEE_SCRIPT);
    let swap = gen_tx_unspent_output(SWAP_SCRIPT);
    let deposit = gen_tx_unspent_output(DEPOSIT_SCRIPT);
    let redeem = gen_tx_unspent_output(REDEEM_SCRIPT);
    ReferenceOutputs {
        pool_v1,
        pool_v2,
        fee_switch_pool,
        fee_switch_pool_bidirectional_fee,
        swap,
        deposit,
        redeem,
    }
}

fn gen_collateral() -> Collateral {
    let script_ref: ScriptRef =
        Script::new_plutus_v2(PlutusV2Script::new(hex::decode(POOL_V2_SCRIPT).unwrap()));
    let tx = ConwayFormatTxOut {
        address: compute_enterprise_addr(script_ref),
        amount: Value::new(50_000_000, MultiAsset::new()),
        datum_option: None,
        script_reference: None,
        encodings: None,
    };
    let utxo_info = TransactionOutput::ConwayFormatTxOut(tx);
    let res = InputBuilderResult {
        input: gen_transaction_input(0),
        utxo_info,
        aggregate_witness: None,
        required_wits: RequiredWitnessSet::default(),
    };

    Collateral(res)
}
