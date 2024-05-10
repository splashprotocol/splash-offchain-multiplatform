use std::{
    sync::{Arc, Once},
    time::UNIX_EPOCH,
};

use async_trait::async_trait;
use bounded_integer::BoundedU8;
use cardano_chain_sync::{
    cache::LedgerCacheRocksDB, chain_sync_stream, client::ChainSyncClient, event_source::ledger_transactions,
};
use cardano_explorer::{client::Explorer, CardanoNetwork, Maestro, PaymentCredential};
use cardano_submit_api::client::LocalTxSubmissionClient;
use chrono::Duration;
use clap::Parser;
use cml_chain::{
    address::Address,
    assets::{AssetName, MultiAsset},
    builders::{
        input_builder::{InputBuilderResult, SingleInputBuilder},
        mint_builder::SingleMintBuilder,
        output_builder::{SingleOutputBuilderResult, TransactionOutputBuilder},
        redeemer_builder::RedeemerWitnessKey,
        tx_builder::{ChangeSelectionAlgo, TransactionUnspentOutput},
        witness_builder::PartialPlutusWitness,
    },
    crypto::TransactionHash,
    plutus::{ConstrPlutusData, ExUnits, PlutusData, RedeemerTag},
    transaction::{DatumOption, Transaction},
    utils::BigInteger,
    Value,
};
use config::AppConfig;
use spectrum_cardano_lib::{
    collateral::Collateral,
    constants::BABBAGE_ERA_ID,
    plutus_data::{ConstrPlutusDataExtension, PlutusDataExtension},
    protocol_params::constant_tx_builder,
    transaction::TransactionOutputExtension,
    OutputRef,
};
use spectrum_offchain::{
    backlog::{persistence::BacklogStoreRocksDB, BacklogConfig, PersistentPriorityBacklog},
    data::Has,
    rocks::RocksConfig,
};
use spectrum_offchain_cardano::{
    constants::MIN_SAFE_COLLATERAL,
    creds::operator_creds,
    tx_submission::{tx_submission_agent_stream, TxSubmissionAgent},
};
use splash_dao_offchain::{
    deployment::{DaoDeployment, DeployedValidators, ProtocolDeployment},
    entities::offchain::voting_order::VotingOrder,
    routines::inflation::Behaviour,
    state_projection::StateProjectionRocksDB,
    time::{NetworkTime, NetworkTimeProvider},
};
use tokio::sync::Mutex;
use tracing::info;
use tracing_subscriber::fmt::Subscriber;

mod config;

#[tokio::main]
async fn main() {
    let subscriber = Subscriber::new();
    tracing::subscriber::set_global_default(subscriber).expect("setting tracing default failed");
    let args = AppArgs::parse();
    let raw_config = std::fs::read_to_string(args.config_path).expect("Cannot load configuration file");
    let config: AppConfig = serde_json::from_str(&raw_config).expect("Invalid configuration file");

    let raw_deployment = std::fs::read_to_string(args.deployment_path).expect("Cannot load deployment file");
    let deployment: DaoDeployment = serde_json::from_str(&raw_deployment).expect("Invalid deployment file");

    log4rs::init_file(args.log4rs_path, Default::default()).unwrap();

    info!("Starting DAO Agent ..");

    let explorer = Maestro::new(config.maestro_key_path, config.network_id.into())
        .await
        .expect("Maestro instantiation failed");
    let farm_factory_unspent_output = explorer
        .utxo_by_ref(OutputRef::new(
            TransactionHash::from_hex("55d96fc0247a4eedb8df071334db35c7dbffea380d914186a229fc1c3aab563e")
                .unwrap(),
            2,
        ))
        .await
        .expect("bbbb");
    let protocol_deployment = ProtocolDeployment::unsafe_pull(deployment.validators, &explorer).await;

    let chain_sync_cache = Arc::new(Mutex::new(LedgerCacheRocksDB::new(config.chain_sync.db_path)));
    let chain_sync = ChainSyncClient::init(
        Arc::clone(&chain_sync_cache),
        config.node.path,
        config.node.magic,
        config.chain_sync.starting_point,
    )
    .await
    .expect("ChainSync initialization failed");

    // n2c clients:
    let tx_submission_client =
        LocalTxSubmissionClient::<BABBAGE_ERA_ID, Transaction>::init(config.node.path, config.node.magic)
            .await
            .expect("LocalTxSubmission initialization failed");
    let (tx_submission_agent, tx_submission_channel) =
        TxSubmissionAgent::new(tx_submission_client, config.tx_submission_buffer_size);

    // prepare upstreams
    let tx_submission_stream = tx_submission_agent_stream(tx_submission_agent);
    let signal_tip_reached = Once::new();
    let ledger_stream = Box::pin(ledger_transactions(
        chain_sync_cache,
        chain_sync_stream(chain_sync, Some(&signal_tip_reached)),
        config.chain_sync.disable_rollbacks_until,
    ));

    let (operator_sk, operator_pkh, operator_addr) =
        operator_creds(config.batcher_private_key, config.node.magic);

    let operator_pkh = operator_pkh.to_bech32("addr_test").unwrap().into();

    let collateral = pull_collateral(operator_pkh, &explorer)
        .await
        .expect("Couldn't retrieve collateral");

    let mut tx_builder = constant_tx_builder();
    let farm_factory_script = PartialPlutusWitness::new(
        cml_chain::builders::witness_builder::PlutusScriptWitness::Ref(protocol_deployment.farm_factory.hash),
        cml_chain::plutus::PlutusData::ConstrPlutusData(ConstrPlutusData::new(0, vec![])),
    );
    let mut farm_factory_out = farm_factory_unspent_output.output.clone();

    let farm_factory_input = SingleInputBuilder::new(
        farm_factory_unspent_output.input,
        farm_factory_unspent_output.output,
    )
    .plutus_script_inline_datum(farm_factory_script, vec![])
    .unwrap();
    tx_builder.add_reference_input(protocol_deployment.farm_factory.reference_utxo);
    tx_builder.add_reference_input(protocol_deployment.smart_farm.reference_utxo);
    tx_builder.add_input(farm_factory_input).unwrap();

    // MintAction {factory_in_ix: 0}
    let mint_redeemer = PlutusData::ConstrPlutusData(ConstrPlutusData::new(
        0,
        vec![PlutusData::new_integer(BigInteger::from(0))],
    ));
    let mint_farm_auth_token_script = PartialPlutusWitness::new(
        cml_chain::builders::witness_builder::PlutusScriptWitness::Ref(protocol_deployment.smart_farm.hash),
        mint_redeemer,
    );
    const EX_UNITS: ExUnits = ExUnits {
        mem: 500_000,
        steps: 200_000_000,
        encodings: None,
    };
    let mint_farm_auth_token =
        SingleMintBuilder::new_single_asset(AssetName::try_from(b"a4".to_vec()).unwrap(), 1)
            .plutus_script(mint_farm_auth_token_script, vec![]);
    tx_builder.add_mint(mint_farm_auth_token).unwrap();

    // farm_factory out
    let farm_factory_value = farm_factory_out.value_mut();
    farm_factory_value.coin = 10_000_000;

    // Increment farm id for farm_factory output's datum
    let farm_auth_token_name = if let Some(datum) = farm_factory_out.data_mut() {
        let cpd = datum.get_constr_pd_mut().unwrap();
        let new_farm_id = if let PlutusData::Integer(i) = cpd.take_field(0).unwrap() {
            i.as_u64().unwrap() + 1
        } else {
            panic!("expected bigint for farm id");
        };

        cpd.set_field(0, PlutusData::new_integer(BigInteger::from(new_farm_id)));
        let mut buffer = [0u8; 128];
        minicbor::encode(new_farm_id, buffer.as_mut()).unwrap();
        AssetName::try_from(buffer.to_vec()).unwrap()
    } else {
        panic!("expected datum for farm_factory input!")
    };
    tx_builder
        .add_output(SingleOutputBuilderResult::new(farm_factory_out))
        .unwrap();

    // smart_farm_out

    let smart_farm_amount = {
        let mut input_multiasset = MultiAsset::new();
        input_multiasset.set(protocol_deployment.smart_farm.hash, farm_auth_token_name, 1);
        Value::new(10_000_000, input_multiasset)
    };

    tx_builder
        .add_output(
            TransactionOutputBuilder::new()
                .with_address(
                    Address::from_bech32(
                        &protocol_deployment
                            .smart_farm
                            .hash
                            .to_bech32("addr_test")
                            .unwrap(),
                    )
                    .unwrap(),
                )
                .with_data(DatumOption::new_datum(PlutusData::new_bytes(vec![])))
                .next()
                .unwrap()
                .with_value(smart_farm_amount)
                .build()
                .unwrap(),
        )
        .unwrap();

    tx_builder.set_exunits(RedeemerWitnessKey::new(RedeemerTag::Spend, 0), EX_UNITS);
    tx_builder.set_exunits(RedeemerWitnessKey::new(RedeemerTag::Mint, 0), EX_UNITS);

    tx_builder
        .add_collateral(InputBuilderResult::from(collateral))
        .unwrap();
    let estimated_tx_fee = tx_builder.min_fee(true).unwrap();
    tx_builder.set_fee(estimated_tx_fee + 1000);

    let signed_tx_builder = tx_builder
        .build(ChangeSelectionAlgo::Default, &operator_addr)
        .unwrap();
    let tx_body = signed_tx_builder.body();
    //let behaviour = Behaviour::new(
    //    StateProjectionRocksDB::new(config.inflation_box_persistence_config),
    //    StateProjectionRocksDB::new(config.poll_factory_persistence_config),
    //    StateProjectionRocksDB::new(config.voting_escrow_persistence_config),
    //    StateProjectionRocksDB::new(config.smart_farm_persistence_config),
    //    StateProjectionRocksDB::new(config.perm_manager_persistence_config),
    //    setup_order_backlog(config.order_backlog_config),
    //    NetworkTimeSource {},
    //);
}

async fn setup_order_backlog(
    store_conf: RocksConfig,
) -> PersistentPriorityBacklog<VotingOrder, BacklogStoreRocksDB> {
    let store = BacklogStoreRocksDB::new(store_conf);
    let backlog_config = BacklogConfig {
        order_lifespan: Duration::try_hours(1).unwrap(),
        order_exec_time: Duration::try_minutes(5).unwrap(),
        retry_suspended_prob: BoundedU8::new(60).unwrap(),
    };

    PersistentPriorityBacklog::new::<VotingOrder>(store, backlog_config).await
}

#[derive(Parser)]
#[command(name = "splash-dao-agent")]
#[command(author = "Spectrum Labs")]
#[command(version = "1.0.0")]
#[command(about = "Splash DAO Agent", long_about = None)]
struct AppArgs {
    /// Path to the JSON configuration file.
    #[arg(long, short)]
    config_path: String,
    /// Path to the JSON deployment configuration file .
    #[arg(long, short)]
    deployment_path: String,
    /// Path to the log4rs YAML configuration file.
    #[arg(long, short)]
    log4rs_path: String,
}

struct NetworkTimeSource;

#[async_trait]
impl NetworkTimeProvider for NetworkTimeSource {
    async fn network_time(&self) -> NetworkTime {
        std::time::SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs()
    }
}
