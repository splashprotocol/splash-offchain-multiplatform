mod mint_token;

use std::ops::Deref;

use cardano_explorer::{CardanoNetwork, Maestro};
use clap::{command, Parser};
use cml_chain::{
    address::Address,
    builders::{
        input_builder::{InputBuilderResult, SingleInputBuilder},
        output_builder::TransactionOutputBuilder,
        tx_builder::{ChangeSelectionAlgo, TransactionUnspentOutput},
    },
    Serialize, Value,
};
use cml_crypto::{Ed25519KeyHash, ScriptHash, TransactionHash};
use mint_token::LQ_NAME;
use spectrum_cardano_lib::{
    collateral::Collateral, protocol_params::constant_tx_builder, transaction::TransactionOutputExtension,
    NetworkId, OutputRef, Token,
};
use spectrum_offchain::tx_prover::TxProver;
use spectrum_offchain_cardano::{
    creds::{operator_creds_base_address, CollateralAddress},
    prover::operator::OperatorProver,
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

    let pk_bech32 = operator_sk.to_bech32();
    println!("pk_bech32: {}", pk_bech32);
    let prover = OperatorProver::new(pk_bech32);

    let collateral = if let Some(c) = pull_collateral(addr.clone().into(), &explorer).await {
        c
    } else {
        generate_collateral(&explorer, &addr, &prover).await.unwrap()
    };

    let operator_pkh_str: String = operator_pkh.into();

    let pk_hash = Ed25519KeyHash::from_bech32(&operator_pkh_str).unwrap();

    let input_result = get_largest_utxo(&explorer, &addr).await;

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
        explorer.wait_for_transaction_confirmation(tx_hash).await.unwrap();

        deployment_config.lq_tokens = Some(minted_token);
        write_deployment_to_disk(&deployment_config, config.deployment_json_path).await;
    }

    if deployment_config.splash_tokens.is_none() {
        println!("Minting SPLASH tokens ----------------------------------------------------------");

        let input_result = get_largest_utxo(&explorer, &addr).await;

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
        explorer.wait_for_transaction_confirmation(tx_hash).await.unwrap();

        deployment_config.splash_tokens = Some(minted_token);
        write_deployment_to_disk(&deployment_config, config.deployment_json_path).await;
    }

    let need_create_token_inputs = deployment_config.nft_utxo_inputs.is_none()
        || deployment_config
            .nft_utxo_inputs
            .as_ref()
            .unwrap()
            .inputs_consumed;

    if need_create_token_inputs {
        println!("Creating inputs to mint deployment tokens ---------------------------------------");
        let input_result = get_largest_utxo(&explorer, &addr).await;
        println!("input ADA: {}", input_result.utxo_info.amount().coin);
        let signed_tx_builder = mint_token::create_minting_tx_inputs(input_result, &addr);
        let tx = prover.prove(signed_tx_builder);
        let tx_hash = TransactionHash::from_hex(&tx.deref().body.hash().to_hex()).unwrap();
        println!("tx_hash: {:?}", tx_hash);
        let tx_bytes = tx.deref().to_cbor_bytes();
        println!("tx_bytes: {}", hex::encode(&tx_bytes));

        explorer.submit_tx(&tx_bytes).await.unwrap();
        explorer.wait_for_transaction_confirmation(tx_hash).await.unwrap();
        if let Some(ref mut i) = deployment_config.nft_utxo_inputs {
            i.tx_hash = tx_hash;
            i.inputs_consumed = false;
        } else {
            deployment_config.nft_utxo_inputs = Some(NFTUtxoInputs {
                tx_hash,
                number_of_inputs: mint_token::NUMBER_TOKEN_MINTS_NEEDED,
                inputs_consumed: false,
            });
            write_deployment_to_disk(&deployment_config, config.deployment_json_path).await;
        }
    }

    if deployment_config.minted_deployment_tokens.is_none() {
        println!("Minting deployment tokens ---------------------------------------");
        let mint_input_tx_hash = deployment_config.nft_utxo_inputs.as_ref().unwrap().tx_hash;
        let inputs = explorer
            .utxos_by_address(addr.clone(), 0, 100)
            .await
            .into_iter()
            .filter_map(|unspent_output| {
                if unspent_output.input.transaction_id == mint_input_tx_hash {
                    return Some(
                        SingleInputBuilder::new(unspent_output.input, unspent_output.output)
                            .payment_key()
                            .unwrap(),
                    );
                }
                None
            })
            .collect();
        let (signed_tx_builder, minted_tokens) =
            mint_token::mint_deployment_tokens(inputs, &addr, pk_hash, collateral);
        let tx = prover.prove(signed_tx_builder);
        let tx_hash = TransactionHash::from_hex(&tx.deref().body.hash().to_hex()).unwrap();
        println!("tx_hash: {:?}", tx_hash);
        let tx_bytes = tx.deref().to_cbor_bytes();
        println!("tx_bytes: {}", hex::encode(&tx_bytes));

        explorer.submit_tx(&tx_bytes).await.unwrap();
        explorer.wait_for_transaction_confirmation(tx_hash).await.unwrap();

        deployment_config.minted_deployment_tokens = Some(minted_tokens);
        deployment_config
            .nft_utxo_inputs
            .as_mut()
            .unwrap()
            .inputs_consumed = true;
        write_deployment_to_disk(&deployment_config, config.deployment_json_path).await;
    }
}

pub async fn get_largest_utxo(explorer: &Maestro, addr: &Address) -> InputBuilderResult {
    let mut utxos = explorer.utxos_by_address(addr.clone(), 0, 50).await;
    utxos.sort_by_key(|output| output.output.value().coin);
    println!("UTxOs: {:?}", utxos.last());

    let utxo = utxos.last().cloned().unwrap();

    SingleInputBuilder::new(utxo.input, utxo.output)
        .payment_key()
        .unwrap()
}

async fn write_deployment_to_disk(deployment_config: &PreprodDeploymentProgress, deployment_json_path: &str) {
    let mut file = tokio::fs::File::create(deployment_json_path).await.unwrap();
    file.write_all((serde_json::to_string(deployment_config).unwrap()).as_bytes())
        .await
        .unwrap();
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
    inputs_consumed: bool,
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

const LIMIT: u16 = 50;
const COLLATERAL_LOVELACES: u64 = 5_000_000;

pub async fn pull_collateral<Net: CardanoNetwork>(
    collateral_address: CollateralAddress,
    explorer: &Net,
) -> Option<Collateral> {
    let mut collateral: Option<TransactionUnspentOutput> = None;
    let mut offset = 0u32;
    let mut num_utxos_pulled = 0;
    while collateral.is_none() {
        let utxos = explorer
            .utxos_by_address(collateral_address.clone().address(), offset, LIMIT)
            .await;
        println!("pull_collateral utxos: {:?}", utxos);
        if utxos.is_empty() {
            break;
        }
        if utxos.len() > num_utxos_pulled {
            num_utxos_pulled = utxos.len();
        } else {
            // Didn't find any new UTxOs
            break;
        }
        if let Some(x) = utxos
            .into_iter()
            .find(|u| !u.output.amount().has_multiassets() && u.output.value().coin == COLLATERAL_LOVELACES)
        {
            collateral = Some(x);
        }
        offset += LIMIT as u32;
    }
    collateral.map(|out| out.into())
}

async fn generate_collateral(
    explorer: &Maestro,
    addr: &Address,
    prover: &OperatorProver,
) -> Result<Collateral, Box<dyn std::error::Error>> {
    let input_utxo = get_largest_utxo(explorer, addr).await;
    let mut tx_builder = constant_tx_builder();
    tx_builder.add_input(input_utxo).unwrap();
    let output_result = TransactionOutputBuilder::new()
        .with_address(addr.clone())
        .next()
        .unwrap()
        .with_value(Value::from(COLLATERAL_LOVELACES))
        .build()
        .unwrap();
    tx_builder.add_output(output_result).unwrap();
    let signed_tx_builder = tx_builder.build(ChangeSelectionAlgo::Default, addr).unwrap();

    let tx = prover.prove(signed_tx_builder);
    let tx_hash = TransactionHash::from_hex(&tx.deref().body.hash().to_hex()).unwrap();
    println!("Generating collateral TX ----------------------------------------------");
    println!("tx_hash: {:?}", tx_hash);
    let tx_bytes = tx.deref().to_cbor_bytes();
    println!("tx_bytes: {}", hex::encode(&tx_bytes));

    explorer.submit_tx(&tx_bytes).await?;
    explorer.wait_for_transaction_confirmation(tx_hash).await?;

    let output_ref = OutputRef::new(tx_hash, 0);
    let utxo = explorer.utxo_by_ref(output_ref).await.unwrap();
    Ok(Collateral::from(utxo))
}
