use cml_chain::builders::tx_builder::TransactionUnspentOutput;
use cml_chain::plutus::{ExUnits, PlutusV1Script, PlutusV2Script, PlutusV3Script};
use cml_chain::transaction::TransactionOutput;
use cml_crypto::{ScriptHash, TransactionHash};
use derive_more::{From, Into};
use hex::FromHexError;
use serde::Deserialize;

use cardano_explorer::client::Explorer;
use spectrum_cardano_lib::OutputRef;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ScriptType {
    PlutusV1,
    PlutusV2,
    PlutusV3,
}

#[derive(Deserialize, Into, From)]
#[serde(try_from = "String")]
pub struct RawScript(Vec<u8>);

impl TryFrom<String> for RawScript {
    type Error = FromHexError;
    fn try_from(string: String) -> Result<Self, Self::Error> {
        hex::decode(string).map(RawScript)
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Script {
    #[serde(rename = "type")]
    pub typ: ScriptType,
    pub script: RawScript,
}

impl From<Script> for cml_chain::Script {
    fn from(value: Script) -> Self {
        match value.typ {
            ScriptType::PlutusV1 => cml_chain::Script::new_plutus_v1(PlutusV1Script::new(value.script.into())),
            ScriptType::PlutusV2 => cml_chain::Script::new_plutus_v2(PlutusV2Script::new(value.script.into())),
            ScriptType::PlutusV3 => cml_chain::Script::new_plutus_v3(PlutusV3Script::new(value.script.into())),
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReferenceUTxO {
    pub tx_hash: TransactionHash,
    pub output_index: u64,
}

impl From<ReferenceUTxO> for OutputRef {
    fn from(value: ReferenceUTxO) -> Self {
        Self::new(value.tx_hash, value.output_index)
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
#[derive(Debug, Clone)]
pub struct ExBudget {
    pub mem: u64,
    pub steps: u64,
}

impl From<ExBudget> for ExUnits {
    fn from(value: ExBudget) -> Self {
        Self {
            mem: value.mem,
            steps: value.steps,
            encodings: None,
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeployedValidatorRef {
    pub script: Script,
    pub hash: ScriptHash,
    pub reference_utxo: ReferenceUTxO,
    pub ex_budget: ExBudget,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeployedValidators {
    pub limit_order_witness: DeployedValidatorRef,
    pub limit_order: DeployedValidatorRef,
    pub const_fn_pool_v1: DeployedValidatorRef,
    pub const_fn_pool_v2: DeployedValidatorRef,
    pub const_fn_pool_fee_switch: DeployedValidatorRef,
    pub const_fn_pool_fee_switch_bidir_fee: DeployedValidatorRef,
    pub const_fn_pool_swap: DeployedValidatorRef,
    pub const_fn_pool_deposit: DeployedValidatorRef,
    pub const_fn_pool_redeem: DeployedValidatorRef,
    pub balance_fn_pool_v1: DeployedValidatorRef,
    pub balance_fn_pool_swap: DeployedValidatorRef,
    pub balance_fn_pool_deposit: DeployedValidatorRef,
    pub balance_fn_pool_redeem: DeployedValidatorRef,
}

#[derive(Debug, Clone)]
pub struct DeployedValidator<const TYP: u8> {
    pub reference_utxo: TransactionUnspentOutput,
    pub hash: ScriptHash,
    pub ex_budget: ExBudget,
}

impl<const TYP: u8> DeployedValidator<TYP> {
    pub fn erased(self) -> DeployedValidatorErased {
        DeployedValidatorErased {
            reference_utxo: self.reference_utxo,
            hash: self.hash,
            ex_budget: self.ex_budget,
        }
    }
}

#[derive(Debug, Clone)]
pub struct DeployedValidatorErased {
    pub reference_utxo: TransactionUnspentOutput,
    pub hash: ScriptHash,
    pub ex_budget: ExBudget,
}

impl<const TYP: u8> DeployedValidator<TYP> {
    async fn unsafe_pull<'a>(v: DeployedValidatorRef, explorer: &Explorer<'a>) -> Self {
        let mut ref_output: TransactionUnspentOutput = explorer
            .get_utxo(v.reference_utxo.into())
            .await
            .expect("Reference UTxO not found")
            .try_into()
            .ok()
            .unwrap();
        match &mut ref_output.output {
            TransactionOutput::AlonzoFormatTxOut(_) => {}
            TransactionOutput::ConwayFormatTxOut(ref mut tx_out) => {
                tx_out.script_reference = Some(v.script.into())
            }
        }
        Self {
            reference_utxo: ref_output,
            hash: v.hash,
            ex_budget: v.ex_budget,
        }
    }
}

#[repr(u8)]
#[derive(Eq, PartialEq)]
pub enum ProtocolValidator {
    LimitOrderWitness,
    LimitOrder,
    ConstFnPoolV1,
    ConstFnPoolV2,
    ConstFnPoolFeeSwitch,
    ConstFnPoolFeeSwitchBiDirFee,
    ConstFnPoolSwap,
    ConstFnPoolDeposit,
    ConstFnPoolRedeem,
    BalanceFnPoolV1,
    BalanceFnPoolSwap,
    BalanceFnPoolDeposit,
    BalanceFnPoolRedeem,
}

#[derive(Debug, Clone)]
pub struct ProtocolDeployment {
    pub limit_order_witness: DeployedValidator<{ ProtocolValidator::LimitOrderWitness as u8 }>,
    pub limit_order: DeployedValidator<{ ProtocolValidator::LimitOrder as u8 }>,
    pub const_fn_pool_v1: DeployedValidator<{ ProtocolValidator::ConstFnPoolV1 as u8 }>,
    pub const_fn_pool_v2: DeployedValidator<{ ProtocolValidator::ConstFnPoolV2 as u8 }>,
    pub const_fn_pool_fee_switch: DeployedValidator<{ ProtocolValidator::ConstFnPoolFeeSwitch as u8 }>,
    pub const_fn_pool_fee_switch_bidir_fee:
        DeployedValidator<{ ProtocolValidator::ConstFnPoolFeeSwitchBiDirFee as u8 }>,
    pub const_fn_pool_swap: DeployedValidator<{ ProtocolValidator::ConstFnPoolSwap as u8 }>,
    pub const_fn_pool_deposit: DeployedValidator<{ ProtocolValidator::ConstFnPoolDeposit as u8 }>,
    pub const_fn_pool_redeem: DeployedValidator<{ ProtocolValidator::ConstFnPoolRedeem as u8 }>,
    pub balance_fn_pool_v1: DeployedValidator<{ ProtocolValidator::BalanceFnPoolV1 as u8 }>,
    pub balance_fn_pool_deposit: DeployedValidator<{ ProtocolValidator::BalanceFnPoolDeposit as u8 }>,
    pub balance_fn_pool_redeem: DeployedValidator<{ ProtocolValidator::BalanceFnPoolRedeem as u8 }>,
}

impl ProtocolDeployment {
    pub async fn unsafe_pull<'a>(validators: DeployedValidators, explorer: &Explorer<'a>) -> Self {
        Self {
            limit_order_witness: DeployedValidator::unsafe_pull(validators.limit_order_witness, &explorer)
                .await,
            limit_order: DeployedValidator::unsafe_pull(validators.limit_order, &explorer).await,
            const_fn_pool_v1: DeployedValidator::unsafe_pull(validators.const_fn_pool_v1, &explorer).await,
            const_fn_pool_v2: DeployedValidator::unsafe_pull(validators.const_fn_pool_v2, &explorer).await,
            const_fn_pool_fee_switch: DeployedValidator::unsafe_pull(
                validators.const_fn_pool_fee_switch,
                &explorer,
            )
            .await,
            const_fn_pool_fee_switch_bidir_fee: DeployedValidator::unsafe_pull(
                validators.const_fn_pool_fee_switch_bidir_fee,
                &explorer,
            )
            .await,
            const_fn_pool_swap: DeployedValidator::unsafe_pull(validators.const_fn_pool_swap, &explorer)
                .await,
            const_fn_pool_deposit: DeployedValidator::unsafe_pull(
                validators.const_fn_pool_deposit,
                &explorer,
            )
            .await,
            const_fn_pool_redeem: DeployedValidator::unsafe_pull(validators.const_fn_pool_redeem, &explorer)
                .await,
            balance_fn_pool_v1: DeployedValidator::unsafe_pull(validators.balance_fn_pool_v1, &explorer)
                .await,
            balance_fn_pool_deposit: DeployedValidator::unsafe_pull(
                validators.balance_fn_pool_deposit,
                &explorer,
            )
            .await,
            balance_fn_pool_redeem: DeployedValidator::unsafe_pull(
                validators.balance_fn_pool_redeem,
                &explorer,
            )
            .await,
        }
    }
}

pub trait RequiresValidator<Ctx> {
    fn get_validator(&self, ctx: &Ctx) -> DeployedValidatorErased;
}
