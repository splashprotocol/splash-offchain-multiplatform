use std::sync::{Arc, Once};

use async_stream::stream;
use cml_chain::{NetworkId, PolicyId, Script, Value};
use cml_chain::address::{Address, EnterpriseAddress};
use cml_chain::assets::MultiAsset;
use cml_chain::builders::input_builder::InputBuilderResult;
use cml_chain::builders::tx_builder::TransactionUnspentOutput;
use cml_chain::builders::witness_builder::RequiredWitnessSet;
use cml_chain::certs::StakeCredential;
use cml_chain::genesis::network_info::NetworkInfo;
use cml_chain::plutus::PlutusV2Script;
use cml_chain::transaction::{
    ConwayFormatTxOut, ScriptRef, Transaction, TransactionInput, TransactionOutput,
};
use cml_chain::transaction::cbor_encodings::TransactionInputEncoding;
use cml_core::network::ProtocolMagic;
use cml_crypto::TransactionHash;
use cml_multi_era::babbage::{BabbageTransaction, BabbageTransactionWitnessSet};
use either::Either;
use futures::{Stream, StreamExt};
use futures::channel::mpsc;
use futures::stream::select_all;
use log::{info, trace};
use rand::Rng;
use tokio::sync::Mutex;
use tracing_subscriber::fmt::Subscriber;

use bloom_cardano_agent::config::AppConfig;
use bloom_cardano_agent::context::ExecutionContext;
use bloom_offchain::execution_engine::bundled::Bundled;
use bloom_offchain::execution_engine::execution_part_stream;
use bloom_offchain::execution_engine::liquidity_book::{ExecutionCap, TLB};
use bloom_offchain::execution_engine::multi_pair::MultiPair;
use bloom_offchain::execution_engine::storage::InMemoryStateIndex;
use bloom_offchain::execution_engine::storage::kv_store::InMemoryKvStore;
use bloom_offchain_cardano::event_sink::entity_index::InMemoryEntityIndex;
use bloom_offchain_cardano::event_sink::EvolvingCardanoEntity;
use bloom_offchain_cardano::event_sink::handler::PairUpdateHandler;
use bloom_offchain_cardano::execution_engine::interpreter::CardanoRecipeInterpreter;
use bloom_offchain_cardano::orders::AnyOrder;
use bloom_offchain_cardano::pools::AnyPool;
use cardano_chain_sync::data::LedgerTxEvent;
use spectrum_cardano_lib::collateral::Collateral;
use spectrum_cardano_lib::output::FinalizedTxOut;
use spectrum_cardano_lib::OutputRef;
use spectrum_offchain::data::Baked;
use spectrum_offchain::data::unique_entity::{EitherMod, StateUpdate};
use spectrum_offchain::event_sink::event_handler::EventHandler;
use spectrum_offchain::event_sink::process_events;
use spectrum_offchain::network::Network;
use spectrum_offchain::partitioning::Partitioned;
use spectrum_offchain::streaming::boxed;
use spectrum_offchain_cardano::constants::{
    DEPOSIT_SCRIPT, FEE_SWITCH_POOL_SCRIPT, FEE_SWITCH_POOL_SCRIPT_BIDIRECTIONAL_FEE_SCRIPT, POOL_V1_SCRIPT,
    POOL_V2_SCRIPT, REDEEM_SCRIPT, SPOT_BATCH_VALIDATOR_SCRIPT, SPOT_SCRIPT, SWAP_SCRIPT,
};
use spectrum_offchain_cardano::creds::operator_creds;
use spectrum_offchain_cardano::data::pair::PairId;
use spectrum_offchain_cardano::data::ref_scripts::ReferenceOutputs;
use spectrum_offchain_cardano::prover::operator::OperatorProver;
use spectrum_offchain_cardano::tx_submission::TxRejected;
use test_utils::babbage::to_babbage_transaction;
use test_utils::pool::gen_pool_transaction_body;
use test_utils::spot::gen_spot_tx;

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
    let pool_lovelaces = 101_000_000;
    let y_token_quantity = 900_000_000;
    let (pool_tx, token_policy, token_name) =
        gen_pool_babbage_tx(NetworkId::new(n_id), pool_lovelaces, y_token_quantity);
    let slot = 49635947;
    let min_tradable_lovelaces = 2_300_000;
    let max_tradable_lovelaces = 20_000_000;
    let ledger_stream = Box::pin(
        futures::stream::iter(vec![
            LedgerTxEvent::TxApplied { tx: pool_tx, slot },
            gen_spot_tx(
                NetworkId::new(n_id),
                token_policy,
                token_name,
                slot + 100,
                2,
                min_tradable_lovelaces,
                max_tradable_lovelaces,
            ),
            gen_spot_tx(
                NetworkId::new(n_id),
                token_policy,
                token_name,
                slot + 200,
                4,
                min_tradable_lovelaces,
                max_tradable_lovelaces / 2,
            ),
        ])
        .chain(stream! {
            let mut slot = slot + 300;
            while let Some(tx) = tx_receiver.recv().await {
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

    let (operator_sk, _, _) = operator_creds(config.batcher_private_key, 1_u64);

    let collateral = gen_collateral();

    let (pair_upd_snd_p1, pair_upd_recv_p1) =
        mpsc::channel::<(PairId, EitherMod<StateUpdate<EvolvingCardanoEntity>>)>(128);
    let (pair_upd_snd_p2, pair_upd_recv_p2) =
        mpsc::channel::<(PairId, EitherMod<StateUpdate<EvolvingCardanoEntity>>)>(128);
    let (pair_upd_snd_p3, pair_upd_recv_p3) =
        mpsc::channel::<(PairId, EitherMod<StateUpdate<EvolvingCardanoEntity>>)>(128);
    let (pair_upd_snd_p4, pair_upd_recv_p4) =
        mpsc::channel::<(PairId, EitherMod<StateUpdate<EvolvingCardanoEntity>>)>(128);

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
    let state_cache = InMemoryKvStore::new();

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

    let start = tokio::time::Instant::now();
    let duration = tokio::time::Duration::from_secs(3);
    loop {
        tokio::select! {
           () =  app.select_next_some() => {

           }
           _ = tokio::time::sleep_until(start + duration) => {
            break;
           }
        }
    }
}

fn unwrap_updates(
    upstream: impl Stream<Item = (PairId, EitherMod<StateUpdate<EvolvingCardanoEntity>>)>,
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
    upstream.map(|(p, m)| (p, m.map(|s| s.map(|EvolvingCardanoEntity(e)| e))))
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

fn gen_pool_babbage_tx(
    network_id: NetworkId,
    lovelaces: u64,
    y_token_quantity: u64,
) -> (BabbageTransaction, PolicyId, &'static str) {
    let (body, token_policy, token_name) = gen_pool_transaction_body(network_id, lovelaces, y_token_quantity);
    let tx = BabbageTransaction::new(body, BabbageTransactionWitnessSet::new(), true, None);
    (tx, token_policy, token_name)
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
    let spot_order = gen_tx_unspent_output(SPOT_SCRIPT);
    let spot_order_batch_validator = gen_tx_unspent_output(SPOT_BATCH_VALIDATOR_SCRIPT);
    ReferenceOutputs {
        pool_v1,
        pool_v2,
        fee_switch_pool,
        fee_switch_pool_bidir_fee: fee_switch_pool_bidirectional_fee,
        swap,
        deposit,
        redeem,
        spot_order,
        spot_order_batch_validator,
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
