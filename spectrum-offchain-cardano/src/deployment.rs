use cml_chain::address::Address;
use cml_chain::builders::tx_builder::TransactionUnspentOutput;
use cml_chain::certs::StakeCredential;
use cml_chain::plutus::{ExUnits, PlutusV1Script, PlutusV2Script, PlutusV3Script};
use cml_chain::transaction::TransactionOutput;
use cml_core::serialization::Deserialize;
use cml_core::DeserializeError;
use cml_crypto::{ScriptHash, TransactionHash};
use derive_more::{From, Into};
use hex::FromHexError;
use std::hash::{Hash, Hasher};
use std::ops::Mul;

use cardano_explorer::client::Explorer;
use spectrum_cardano_lib::OutputRef;
use spectrum_offchain::data::Has;

#[derive(serde::Deserialize)]
#[serde(rename_all = "PascalCase")]
pub enum ScriptType {
    PlutusV1,
    PlutusV2,
    PlutusV3,
}

#[derive(serde::Deserialize, Into, From)]
#[serde(try_from = "String")]
pub struct RawCBORScript(Vec<u8>);

impl TryFrom<String> for RawCBORScript {
    type Error = FromHexError;
    fn try_from(string: String) -> Result<Self, Self::Error> {
        hex::decode(string).map(RawCBORScript)
    }
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Script {
    #[serde(rename = "type")]
    pub typ: ScriptType,
    pub script: RawCBORScript,
}

impl TryFrom<Script> for cml_chain::Script {
    type Error = DeserializeError;
    fn try_from(value: Script) -> Result<Self, Self::Error> {
        Ok(match value.typ {
            ScriptType::PlutusV1 => {
                cml_chain::Script::new_plutus_v1(PlutusV1Script::from_cbor_bytes(&*value.script.0)?)
            }
            ScriptType::PlutusV2 => {
                cml_chain::Script::new_plutus_v2(PlutusV2Script::from_cbor_bytes(&*value.script.0)?)
            }
            ScriptType::PlutusV3 => {
                cml_chain::Script::new_plutus_v3(PlutusV3Script::from_cbor_bytes(&*value.script.0)?)
            }
        })
    }
}

#[derive(serde::Deserialize)]
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

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
#[derive(Debug, Clone)]
pub struct ExBudget {
    pub mem: u64,
    pub steps: u64,
}

impl ExBudget {
    pub fn scale(self, factor: u64) -> Self {
        Self {
            mem: self.mem * factor,
            steps: self.steps * factor,
        }
    }
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

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeployedValidatorRef {
    pub script: Script,
    pub hash: ScriptHash,
    pub reference_utxo: ReferenceUTxO,
    // pub ex_budget: ExBudget,
}

#[derive(serde::Deserialize)]
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
    pub balance_fn_pool_deposit: DeployedValidatorRef,
    pub balance_fn_pool_redeem: DeployedValidatorRef,
}

impl From<&DeployedValidators> for ProtocolScriptHashes {
    fn from(deployment: &DeployedValidators) -> Self {
        Self {
            limit_order_witness: From::from(&deployment.limit_order_witness),
            limit_order: From::from(&deployment.limit_order),
            const_fn_pool_v1: From::from(&deployment.const_fn_pool_v1),
            const_fn_pool_v2: From::from(&deployment.const_fn_pool_v2),
            const_fn_pool_fee_switch: From::from(&deployment.const_fn_pool_fee_switch),
            const_fn_pool_fee_switch_bidir_fee: From::from(&deployment.const_fn_pool_fee_switch_bidir_fee),
            const_fn_pool_swap: From::from(&deployment.const_fn_pool_swap),
            const_fn_pool_deposit: From::from(&deployment.const_fn_pool_deposit),
            const_fn_pool_redeem: From::from(&deployment.const_fn_pool_redeem),
            balance_fn_pool_v1: From::from(&deployment.balance_fn_pool_v1),
            balance_fn_pool_deposit: From::from(&deployment.balance_fn_pool_deposit),
            balance_fn_pool_redeem: From::from(&deployment.balance_fn_pool_redeem),
        }
    }
}

#[repr(transparent)]
#[derive(Debug, Copy, Clone, Into, From)]
pub struct DeployedScriptHash<const TYP: u8>(ScriptHash);

impl<const TYP: u8> DeployedScriptHash<TYP> {
    pub fn unwrap(self) -> ScriptHash {
        self.0
    }
}

pub fn test_address<const TYP: u8, Ctx>(addr: &Address, ctx: &Ctx) -> bool
where
    Ctx: Has<DeployedScriptHash<TYP>>,
{
    let maybe_hash = addr.payment_cred().and_then(|c| match c {
        StakeCredential::PubKey { .. } => None,
        StakeCredential::Script { hash, .. } => Some(hash),
    });
    if let Some(this_hash) = maybe_hash {
        return *this_hash == ctx.get().unwrap();
    }
    false
}

impl<const TYP: u8> From<&DeployedValidator<TYP>> for DeployedScriptHash<TYP> {
    fn from(value: &DeployedValidator<TYP>) -> Self {
        Self(value.hash)
    }
}

impl<const TYP: u8> From<&DeployedValidatorRef> for DeployedScriptHash<TYP> {
    fn from(value: &DeployedValidatorRef) -> Self {
        Self(value.hash)
    }
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

impl Hash for DeployedValidatorErased {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.hash.hash(state)
    }
}

impl PartialEq for DeployedValidatorErased {
    fn eq(&self, other: &Self) -> bool {
        self.hash.eq(&other.hash)
    }
}

impl Eq for DeployedValidatorErased {}

#[derive(Debug, Clone)]
pub struct ScriptWitness {
    pub hash: ScriptHash,
    pub ex_budget: ExBudget,
}

impl<const TYP: u8> DeployedValidator<TYP> {
    pub async fn unsafe_pull<'a>(v: DeployedValidatorRef, explorer: &Explorer<'a>) -> Self {
        let mut ref_output = explorer
            .get_utxo(v.reference_utxo.into())
            .await
            .expect("Reference UTxO from config not found")
            .try_into_cml()
            .unwrap();
        match &mut ref_output.output {
            TransactionOutput::ConwayFormatTxOut(ref mut tx_out) => {
                tx_out.script_reference = Some(v.script.try_into().expect("Invalid script was provided"))
            }
            TransactionOutput::AlonzoFormatTxOut(_) => panic!("Must be ConwayFormatTxOut"),
        }
        let ex_budget = ExBudget {
            mem: 500000,
            steps: 200000000,
        };
        Self {
            reference_utxo: ref_output,
            hash: v.hash,
            ex_budget,
        }
    }
}

#[repr(u8)]
#[derive(Eq, PartialEq)]
pub enum ProtocolValidator {
    LimitOrderWitnessV1,
    LimitOrderV1,
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

#[derive(Debug, Copy, Clone)]
pub struct ProtocolScriptHashes {
    pub limit_order_witness: DeployedScriptHash<{ ProtocolValidator::LimitOrderWitnessV1 as u8 }>,
    pub limit_order: DeployedScriptHash<{ ProtocolValidator::LimitOrderV1 as u8 }>,
    pub const_fn_pool_v1: DeployedScriptHash<{ ProtocolValidator::ConstFnPoolV1 as u8 }>,
    pub const_fn_pool_v2: DeployedScriptHash<{ ProtocolValidator::ConstFnPoolV2 as u8 }>,
    pub const_fn_pool_fee_switch: DeployedScriptHash<{ ProtocolValidator::ConstFnPoolFeeSwitch as u8 }>,
    pub const_fn_pool_fee_switch_bidir_fee:
        DeployedScriptHash<{ ProtocolValidator::ConstFnPoolFeeSwitchBiDirFee as u8 }>,
    pub const_fn_pool_swap: DeployedScriptHash<{ ProtocolValidator::ConstFnPoolSwap as u8 }>,
    pub const_fn_pool_deposit: DeployedScriptHash<{ ProtocolValidator::ConstFnPoolDeposit as u8 }>,
    pub const_fn_pool_redeem: DeployedScriptHash<{ ProtocolValidator::ConstFnPoolRedeem as u8 }>,
    pub balance_fn_pool_v1: DeployedScriptHash<{ ProtocolValidator::BalanceFnPoolV1 as u8 }>,
    pub balance_fn_pool_deposit: DeployedScriptHash<{ ProtocolValidator::BalanceFnPoolDeposit as u8 }>,
    pub balance_fn_pool_redeem: DeployedScriptHash<{ ProtocolValidator::BalanceFnPoolRedeem as u8 }>,
}

impl From<&ProtocolDeployment> for ProtocolScriptHashes {
    fn from(deployment: &ProtocolDeployment) -> Self {
        Self {
            limit_order_witness: From::from(&deployment.limit_order_witness),
            limit_order: From::from(&deployment.limit_order),
            const_fn_pool_v1: From::from(&deployment.const_fn_pool_v1),
            const_fn_pool_v2: From::from(&deployment.const_fn_pool_v2),
            const_fn_pool_fee_switch: From::from(&deployment.const_fn_pool_fee_switch),
            const_fn_pool_fee_switch_bidir_fee: From::from(&deployment.const_fn_pool_fee_switch_bidir_fee),
            const_fn_pool_swap: From::from(&deployment.const_fn_pool_swap),
            const_fn_pool_deposit: From::from(&deployment.const_fn_pool_deposit),
            const_fn_pool_redeem: From::from(&deployment.const_fn_pool_redeem),
            balance_fn_pool_v1: From::from(&deployment.balance_fn_pool_v1),
            balance_fn_pool_deposit: From::from(&deployment.balance_fn_pool_deposit),
            balance_fn_pool_redeem: From::from(&deployment.balance_fn_pool_redeem),
        }
    }
}

#[derive(Debug, Clone)]
pub struct ProtocolDeployment {
    pub limit_order_witness: DeployedValidator<{ ProtocolValidator::LimitOrderWitnessV1 as u8 }>,
    pub limit_order: DeployedValidator<{ ProtocolValidator::LimitOrderV1 as u8 }>,
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
