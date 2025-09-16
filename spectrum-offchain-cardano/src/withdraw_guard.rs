use log::{error, info, warn};
use std::fmt::Display;
use std::time::Duration;
use tokio::time::sleep;

use crate::collateral::pull_collateral;
use crate::creds::CollateralAddress;
use crate::deployment::DeployedValidatorErased;
use cardano_explorer::{CardanoNetwork, ExtendedCardanoNetwork};
use cml_chain::address::{Address, RewardAddress};
use cml_chain::builders::input_builder::SingleInputBuilder;
use cml_chain::builders::redeemer_builder::RedeemerWitnessKey;
use cml_chain::builders::tx_builder::{ChangeSelectionAlgo, SignedTxBuilder, TransactionUnspentOutput};
use cml_chain::builders::withdrawal_builder::SingleWithdrawalBuilder;
use cml_chain::builders::witness_builder::{PartialPlutusWitness, PlutusScriptWitness};
use cml_chain::certs::StakeCredential;
use cml_chain::plutus::{PlutusData, RedeemerTag};
use cml_crypto::{ScriptHash, TransactionHash};
use futures::TryFutureExt;
use spectrum_cardano_lib::collateral::Collateral;
use spectrum_cardano_lib::ex_units::ExUnits;
use spectrum_cardano_lib::hash::hash_transaction_canonical;
use spectrum_offchain::network::Network;
use spectrum_offchain::tx_hash::CanonicalHash;
use spectrum_offchain::tx_prover::TxProver;

#[derive(Copy, Clone, Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WithdrawGuardConfig {
    pub max_attempts: u32,
    pub submission_delay: Duration,
}

#[derive(Debug, thiserror::Error)]
pub enum WithdrawError {
    #[error("Withdrawal creation error: {0}")]
    WithdrawalCreationError(String),

    #[error("Transaction build error: {0}")]
    TransactionBuildError(String),

    #[error("Transaction confirmation error: {0}")]
    TransactionConfirmationError(String),
}

pub fn build_withdrawal_transaction(
    collateral: Collateral,
    reward_address: RewardAddress,
    rewards: u64,
    script_validator: &DeployedValidatorErased,
    change_address: Address,
) -> Result<SignedTxBuilder, WithdrawError> {
    let mut tx_builder = spectrum_cardano_lib::protocol_params::constant_tx_builder();

    let partial_witness = PartialPlutusWitness::new(
        PlutusScriptWitness::Ref(script_validator.hash),
        PlutusData::new_integer(0.into()),
    );

    let withdrawal_result = SingleWithdrawalBuilder::new(reward_address, rewards)
        .plutus_script(partial_witness, vec![].into())
        .map_err(|e| WithdrawError::WithdrawalCreationError(e.to_string()))?;

    tx_builder.add_reference_input(script_validator.reference_utxo.clone());

    tx_builder.add_withdrawal(withdrawal_result);

    tx_builder.set_exunits(
        RedeemerWitnessKey::new(RedeemerTag::Reward, 0),
        script_validator.ex_budget.into(),
    );

    let utxo: TransactionUnspentOutput = collateral.into();
    let collateral_input = SingleInputBuilder::new(utxo.input, utxo.output)
        .payment_key()
        .map_err(|e| WithdrawError::WithdrawalCreationError(e.to_string()))?;

    tx_builder.add_input(collateral_input);

    let tx = tx_builder
        .build(ChangeSelectionAlgo::Default, &change_address)
        .map_err(|e| WithdrawError::TransactionBuildError(e.to_string()))?;

    Ok(tx)
}

pub async fn withdraw_script_guard<E, N, ER>(
    config: WithdrawGuardConfig,
    explorer: &E,
    network: N,
    prover: &impl TxProver<SignedTxBuilder, cml_chain::transaction::Transaction>,
    collateral_address: Address,
    script_validator: &DeployedValidatorErased,
    network_id: u8,
    collateral: Collateral,
) -> Result<(), WithdrawError>
where
    E: CardanoNetwork,
    N: Network<cml_chain::transaction::Transaction, ER> + Clone,
    ER: Display,
{
    let script_reward_address =
        RewardAddress::new(network_id, StakeCredential::new_script(script_validator.hash));

    let mut iterations_qty = 0;

    while iterations_qty < config.max_attempts {
        info!(
            "Verifying rewards on script hash {}",
            script_validator.hash.to_hex()
        );

        let rewards = explorer
            .rewards_by_account(script_reward_address.clone().to_address(), 0, 100)
            .await;

        if rewards == 0 {
            info!("Rewards on script hash {} is 0", script_validator.hash.to_hex());
            return Ok(());
        }

        info!("Found {} lovelace in rewards on script, withdrawing...", rewards);

        info!("Withdrawal attempt {} of {}", iterations_qty, config.max_attempts);

        // Build the transaction
        let tx = match build_withdrawal_transaction(
            collateral.clone(),
            script_reward_address.clone(),
            rewards,
            script_validator,
            collateral_address.clone(),
        ) {
            Ok(tx) => tx,
            Err(err) => {
                error!("Failed to build withdrawal transaction: {}", err);
                sleep(config.submission_delay).await;
                iterations_qty += 1;
                continue;
            }
        };

        let tx_signed = prover.prove(tx.into());

        let mut network = network.clone();

        info!("Submitting withdrawal transaction...");
        match network.submit_tx(tx_signed).await {
            Ok(_) => {
                info!("Withdrawal transaction submitted successfully");
            }
            Err(err) => {
                let error_msg = format!("Failed to submit withdrawal transaction: {}", err);
                error!("{}", error_msg);
                warn!("Withdrawal attempt {} failed: {}", iterations_qty, error_msg);
            }
        };

        sleep(config.submission_delay).await;
        iterations_qty += 1;
    }

    warn!("Maximum withdrawal attempts reached.");
    Err(WithdrawError::TransactionConfirmationError(format!(
        "Failed to withdraw rewards after {} attempts",
        config.max_attempts
    )))
}
