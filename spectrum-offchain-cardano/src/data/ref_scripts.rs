use cml_chain::builders::tx_builder::TransactionUnspentOutput;
use cml_chain::plutus::PlutusV2Script;
use cml_chain::transaction::{ScriptRef, TransactionOutput};
use cml_chain::Script;

use cardano_explorer::client::Explorer;
use spectrum_cardano_lib::OutputRef;
use spectrum_offchain::data::Has;

use crate::constants::{
    BALANCE_POOL_SCRIPT, DEPOSIT_SCRIPT, FEE_SWITCH_POOL_SCRIPT,
    FEE_SWITCH_POOL_SCRIPT_BIDIRECTIONAL_FEE_SCRIPT, LIMIT_ORDER_SCRIPT, POOL_V1_SCRIPT, POOL_V2_SCRIPT,
    REDEEM_SCRIPT, SPOT_BATCH_VALIDATOR_SCRIPT, SWAP_SCRIPT,
};
use crate::data::balance_pool::BalancePool;
use crate::data::cfmm_pool::CFMMPool;
use crate::data::deposit::ClassicalOnChainDeposit;
use crate::data::limit_swap::ClassicalOnChainLimitSwap;
use crate::data::redeem::ClassicalOnChainRedeem;
use crate::data::PoolVer;
use crate::ref_scripts::ReferenceSources;

#[derive(Debug, Clone)]
pub struct ReferenceOutputs {
    pub pool_v1: TransactionUnspentOutput,
    pub pool_v2: TransactionUnspentOutput,
    pub fee_switch_pool: TransactionUnspentOutput,
    pub fee_switch_pool_bidir_fee: TransactionUnspentOutput,
    pub balance_pool: TransactionUnspentOutput,
    pub swap: TransactionUnspentOutput,
    pub deposit: TransactionUnspentOutput,
    pub redeem: TransactionUnspentOutput,
    pub spot_order: TransactionUnspentOutput,
    pub spot_order_batch_validator: TransactionUnspentOutput,
}

impl ReferenceOutputs {
    pub async fn pull<'a>(config: ReferenceSources, explorer: Explorer<'a>) -> Option<ReferenceOutputs> {
        async fn process_utxo_with_ref_script<'a>(
            tx_out: OutputRef,
            raw_ref_script: &str,
            explorer: Explorer<'a>,
        ) -> Option<TransactionUnspentOutput> {
            let previous_output: TransactionUnspentOutput =
                explorer.get_utxo(tx_out).await.unwrap().try_into().ok()?;

            let script_ref: Option<ScriptRef> = Some(Script::new_plutus_v2(PlutusV2Script::new(
                hex::decode(raw_ref_script).unwrap(),
            )));

            let updated_new_output = TransactionOutput::new(
                previous_output.output.address().clone(),
                previous_output.output.amount().clone(),
                previous_output.output.datum(),
                script_ref,
            );

            Some(TransactionUnspentOutput::new(
                previous_output.input,
                updated_new_output,
            ))
        }

        let pool_v1 = process_utxo_with_ref_script(config.pool_v1_script, POOL_V1_SCRIPT, explorer).await?;
        let pool_v2 = process_utxo_with_ref_script(config.pool_v2_script, POOL_V2_SCRIPT, explorer).await?;
        let fee_switch_pool =
            process_utxo_with_ref_script(config.fee_switch_pool_script, FEE_SWITCH_POOL_SCRIPT, explorer)
                .await?;
        let fee_switch_pool_bidirectional_fee = process_utxo_with_ref_script(
            config.fee_switch_pool_bidirectional_fee_script,
            FEE_SWITCH_POOL_SCRIPT_BIDIRECTIONAL_FEE_SCRIPT,
            explorer,
        )
        .await?;
        let balance_pool =
            process_utxo_with_ref_script(config.balance_pool_script, BALANCE_POOL_SCRIPT, explorer).await?;
        let swap = process_utxo_with_ref_script(config.swap_script, SWAP_SCRIPT, explorer).await?;
        let deposit = process_utxo_with_ref_script(config.deposit_script, DEPOSIT_SCRIPT, explorer).await?;
        let redeem = process_utxo_with_ref_script(config.redeem_script, REDEEM_SCRIPT, explorer).await?;
        let spot_order =
            process_utxo_with_ref_script(config.redeem_script, LIMIT_ORDER_SCRIPT, explorer).await?;
        let spot_order_batch_validator =
            process_utxo_with_ref_script(config.redeem_script, SPOT_BATCH_VALIDATOR_SCRIPT, explorer).await?;
        Some(ReferenceOutputs {
            pool_v1,
            pool_v2,
            fee_switch_pool,
            fee_switch_pool_bidir_fee: fee_switch_pool_bidirectional_fee,
            balance_pool,
            swap,
            deposit,
            redeem,
            spot_order,
            spot_order_batch_validator,
        })
    }
}

pub trait RequiresRefScript {
    fn get_ref_script(self, ref_scripts: ReferenceOutputs) -> TransactionUnspentOutput;
}

impl RequiresRefScript for ClassicalOnChainDeposit {
    fn get_ref_script(self, ref_scripts: ReferenceOutputs) -> TransactionUnspentOutput {
        ref_scripts.deposit
    }
}

impl RequiresRefScript for ClassicalOnChainLimitSwap {
    fn get_ref_script(self, ref_scripts: ReferenceOutputs) -> TransactionUnspentOutput {
        ref_scripts.swap
    }
}

impl RequiresRefScript for ClassicalOnChainRedeem {
    fn get_ref_script(self, ref_scripts: ReferenceOutputs) -> TransactionUnspentOutput {
        ref_scripts.redeem
    }
}

impl RequiresRefScript for CFMMPool {
    fn get_ref_script(self, ref_scripts: ReferenceOutputs) -> TransactionUnspentOutput {
        match self.get_labeled::<PoolVer>() {
            PoolVer::V1 => ref_scripts.pool_v1,
            PoolVer::V2 => ref_scripts.pool_v2,
            PoolVer::FeeSwitch => ref_scripts.fee_switch_pool,
            PoolVer::FeeSwitchBiDirFee => ref_scripts.fee_switch_pool_bidir_fee,
            PoolVer::BalancePool => unreachable!(),
        }
    }
}

impl RequiresRefScript for BalancePool {
    fn get_ref_script(self, ref_scripts: ReferenceOutputs) -> TransactionUnspentOutput {
        match self.get_labeled::<PoolVer>() {
            PoolVer::BalancePool => ref_scripts.balance_pool,
            _ => unreachable!(),
        }
    }
}
