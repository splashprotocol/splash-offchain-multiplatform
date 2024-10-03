mod mint_token;

use std::ops::Deref;

use cardano_explorer::{CardanoNetwork, Maestro};
use clap::{command, Parser};
use cml_chain::{builders::input_builder::SingleInputBuilder, Serialize};
use cml_crypto::{Ed25519KeyHash, ScriptHash, TransactionHash};
use mint_token::LQ_NAME;
use spectrum_cardano_lib::{transaction::TransactionOutputExtension, NetworkId, Token};
use spectrum_offchain::tx_prover::TxProver;
use spectrum_offchain_cardano::{
    collateral::pull_collateral, creds::operator_creds_base_address, prover::operator::OperatorProver,
};
use splash_dao_offchain::{
    constants::SPLASH_NAME,
    deployment::{BuiltPolicy, DeployedValidators, MintedTokens},
    entities::onchain::{
        inflation_box::InflationBoxSnapshot, permission_manager::PermManagerSnapshot,
        poll_factory::PollFactorySnapshot,
    },
};
use tokio::io::AsyncWriteExt;

const INFLATION_BOX_INITIAL_SPLASH_QTY: i64 = 32000000000000;

#[tokio::main]
async fn main() {
    let args = AppArgs::parse();
    let raw_config = std::fs::read_to_string(args.config_path).expect("Cannot load configuration file");
    let config: AppConfig = serde_json::from_str(&raw_config).expect("Invalid configuration file");

    deploy(config).await;
}

async fn deploy<'a>(config: AppConfig<'a>) {
    let explorer = Maestro::new(config.maestro_key_path, config.network_id.into())
        .await
        .expect("Maestro instantiation failed");

    let (addr, _, operator_pkh, operator_cred, operator_sk) =
        operator_creds_base_address(config.batcher_private_key, config.network_id);

    let collateral = pull_collateral(addr.clone().into(), &explorer)
        .await
        .expect("Couldn't retrieve collateral");

    let operator_pkh_str: String = operator_pkh.into();

    let pk_hash = Ed25519KeyHash::from_bech32(&operator_pkh_str).unwrap();

    let mut utxos = explorer.utxos_by_address(addr.clone(), 0, 20).await;
    utxos.sort_by_key(|output| output.output.value().coin);
    println!("UTxOs: {:?}", utxos.last());

    let utxo = utxos.last().cloned().unwrap();

    let input_result = SingleInputBuilder::new(utxo.input, utxo.output)
        .payment_key()
        .unwrap();

    let pk_bech32 = operator_sk.to_bech32();
    println!("pk_bech32: {}", pk_bech32);
    let prover = OperatorProver::new(pk_bech32);

    let raw_deployment_config =
        std::fs::read_to_string(config.deployment_json_path).expect("Cannot load configuration file");
    let mut deployment_config: PreprodDeploymentProgress =
        serde_json::from_str(&raw_deployment_config).expect("Invalid configuration file");

    // Mint LQ token -------------------------------------------------------------------------------
    if deployment_config.lq_tokens.is_none() {
        println!("Minting LQ tokens ----------------------------------------------------------");
        let (signed_tx_builder, minted_token) = mint_token::mint_token(
            LQ_NAME,
            INFLATION_BOX_INITIAL_SPLASH_QTY,
            pk_hash,
            input_result,
            &addr,
            explorer.chain_tip_slot_number().await.unwrap(),
        );
        let tx = prover.prove(signed_tx_builder);
        let tx_hash = TransactionHash::from_hex(&tx.deref().body.hash().to_hex()).unwrap();
        println!("tx_hash: {:?}", tx_hash);
        let tx_bytes = tx.deref().to_cbor_bytes();
        println!("tx_bytes: {}", hex::encode(&tx_bytes));

        explorer.submit_tx(&tx_bytes).await.unwrap();
        explorer.wait_for_transaction_confirmation(tx_hash).await.is_ok();

        deployment_config.lq_tokens = Some(minted_token);
        let mut file = tokio::fs::File::create(config.deployment_json_path)
            .await
            .unwrap();
        file.write_all((serde_json::to_string(&deployment_config).unwrap()).as_bytes())
            .await
            .unwrap();
    }

    if deployment_config.splash_tokens.is_none() {
        println!("Minting SPLASH tokens ----------------------------------------------------------");

        let mut utxos = explorer.utxos_by_address(addr.clone(), 0, 20).await;
        utxos.sort_by_key(|output| output.output.value().coin);
        println!("UTxOs: {:?}", utxos.last());

        let utxo = utxos.last().cloned().unwrap();

        let input_result = SingleInputBuilder::new(utxo.input, utxo.output)
            .payment_key()
            .unwrap();
        let (signed_tx_builder, minted_token) = mint_token::mint_token(
            SPLASH_NAME,
            2 * INFLATION_BOX_INITIAL_SPLASH_QTY,
            pk_hash,
            input_result,
            &addr,
            explorer.chain_tip_slot_number().await.unwrap(),
        );
        let tx = prover.prove(signed_tx_builder);
        let tx_hash = TransactionHash::from_hex(&tx.deref().body.hash().to_hex()).unwrap();
        println!("tx_hash: {:?}", tx_hash);
        let tx_bytes = tx.deref().to_cbor_bytes();
        println!("tx_bytes: {}", hex::encode(&tx_bytes));

        explorer.submit_tx(&tx_bytes).await.unwrap();
        explorer.wait_for_transaction_confirmation(tx_hash).await.is_ok();

        deployment_config.splash_tokens = Some(minted_token);
        let mut file = tokio::fs::File::create(config.deployment_json_path)
            .await
            .unwrap();
        file.write_all((serde_json::to_string(&deployment_config).unwrap()).as_bytes())
            .await
            .unwrap();
    }
}

#[derive(Parser)]
#[command(name = "splash-dao-administration")]
#[command(author = "Spectrum Labs")]
#[command(version = "1.0.0")]
#[command(about = "Splash DAO Administration", long_about = None)]
struct AppArgs {
    /// Path to the JSON configuration file.
    #[arg(long, short)]
    config_path: String,
}

#[derive(serde::Deserialize)]
#[serde(bound = "'de: 'a")]
#[serde(rename_all = "camelCase")]
pub struct AppConfig<'a> {
    pub network_id: NetworkId,
    pub maestro_key_path: &'a str,
    pub batcher_private_key: &'a str, //todo: store encrypted
    pub deployment_json_path: &'a str,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct PreprodDeploymentProgress {
    lq_tokens: Option<ExternallyMintedToken>,
    splash_tokens: Option<ExternallyMintedToken>,
    nft_utxo_inputs: Option<NFTUtxoInputs>,
    minted_deployment_tokens: Option<MintedTokens>,
    deployed_validators: Option<DeployedValidators>,
}

#[derive(serde::Serialize, serde::Deserialize, Clone)]
struct ExternallyMintedToken {
    token: Token,
    quantity: u64,
}

#[derive(serde::Serialize, serde::Deserialize, Clone)]
/// Each NFT we mint requires a distinct UTxO input.
struct NFTUtxoInputs {
    tx_hash: TransactionHash,
    number_of_inputs: usize,
}

#[derive(serde::Serialize, serde::Deserialize, Clone)]
struct InitialDaoEntities {
    inflation_box: InflationBoxSnapshot,
    //farm_factory: FarmFactorySn
    weighting_poll_factory: PollFactorySnapshot,
    //voting_escrow_factory: VE
    // gov_proxy: Gov
    permission_manager: PermManagerSnapshot,
}
