use std::fmt::{Display, Formatter};
use std::hash::{Hash, Hasher};

use cml_chain::address::Address;
use cml_chain::builders::tx_builder::TransactionUnspentOutput;
use cml_chain::certs::StakeCredential;
use cml_chain::plutus::{PlutusV1Script, PlutusV2Script, PlutusV3Script};
use cml_core::serialization::Deserialize;
use cml_core::DeserializeError;
use cml_crypto::{ScriptHash, TransactionHash};
use derive_more::{From, Into};
use hex::FromHexError;

use algebra_core::monoid::Monoid;
use cardano_explorer::CardanoNetwork;
use spectrum_cardano_lib::ex_units::ExUnits;
use spectrum_cardano_lib::OutputRef;
use spectrum_offchain::domain::Has;

#[derive(serde::Serialize, serde::Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub enum ScriptType {
    PlutusV1,
    PlutusV2,
    PlutusV3,
}

#[derive(serde::Serialize, serde::Deserialize, Into, From, Clone)]
#[serde(try_from = "String", into = "String")]
pub struct RawCBORScript(Vec<u8>);

impl TryFrom<String> for RawCBORScript {
    type Error = FromHexError;
    fn try_from(string: String) -> Result<Self, Self::Error> {
        hex::decode(string).map(RawCBORScript)
    }
}

impl From<RawCBORScript> for String {
    fn from(value: RawCBORScript) -> Self {
        hex::encode(&value.0)
    }
}

#[derive(serde::Serialize, serde::Deserialize, Clone)]
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

#[derive(Copy, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReferenceUTxO {
    pub tx_hash: TransactionHash,
    pub output_index: u64,
}

impl Display for ReferenceUTxO {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(format!("{}:{}", self.tx_hash, self.output_index).as_str())
    }
}

impl From<ReferenceUTxO> for OutputRef {
    fn from(value: ReferenceUTxO) -> Self {
        Self::new(value.tx_hash, value.output_index)
    }
}

#[derive(serde::Serialize, serde::Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct DeployedValidatorRef {
    pub hash: ScriptHash,
    pub reference_utxo: ReferenceUTxO,
    /// Cost per contract invokation.
    pub cost: ExUnits,
    /// Cost per each subsequent contract invokation.
    /// Consider a batch witness script: first invokation costs `cost`,
    /// each subsequent invokation adds `marginal_cost` to base cost.
    pub marginal_cost: Option<ExUnits>,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeployedValidators {
    pub limit_order_witness: DeployedValidatorRef,
    pub limit_order: DeployedValidatorRef,
    pub grid_order_native: DeployedValidatorRef,
    pub const_fn_pool_v1: DeployedValidatorRef,
    pub const_fn_pool_v2: DeployedValidatorRef,
    pub const_fn_pool_fee_switch: DeployedValidatorRef,
    pub const_fn_pool_fee_switch_v2: DeployedValidatorRef,
    pub const_fn_pool_fee_switch_bidir_fee: DeployedValidatorRef,
    pub const_fn_pool_swap: DeployedValidatorRef,
    pub const_fn_pool_deposit: DeployedValidatorRef,
    pub const_fn_pool_redeem: DeployedValidatorRef,
    pub const_fn_fee_switch_pool_swap: DeployedValidatorRef,
    pub const_fn_fee_switch_pool_deposit: DeployedValidatorRef,
    pub const_fn_fee_switch_pool_redeem: DeployedValidatorRef,
    pub balance_fn_pool_v1: DeployedValidatorRef,
    pub balance_fn_pool_v2: DeployedValidatorRef,
    pub balance_fn_pool_deposit: DeployedValidatorRef,
    pub balance_fn_pool_redeem: DeployedValidatorRef,
    pub stable_fn_pool_t2t: DeployedValidatorRef,
    pub stable_fn_pool_t2t_deposit: DeployedValidatorRef,
    pub stable_fn_pool_t2t_redeem: DeployedValidatorRef,
}

impl From<&DeployedValidators> for ProtocolScriptHashes {
    fn from(deployment: &DeployedValidators) -> Self {
        Self {
            limit_order_witness: From::from(&deployment.limit_order_witness),
            limit_order: From::from(&deployment.limit_order),
            grid_order_native: From::from(&deployment.grid_order_native),
            const_fn_pool_v1: From::from(&deployment.const_fn_pool_v1),
            const_fn_pool_v2: From::from(&deployment.const_fn_pool_v2),
            const_fn_pool_fee_switch: From::from(&deployment.const_fn_pool_fee_switch),
            const_fn_pool_fee_switch_v2: From::from(&deployment.const_fn_pool_fee_switch_v2),
            const_fn_pool_fee_switch_bidir_fee: From::from(&deployment.const_fn_pool_fee_switch_bidir_fee),
            const_fn_pool_swap: From::from(&deployment.const_fn_pool_swap),
            const_fn_pool_deposit: From::from(&deployment.const_fn_pool_deposit),
            const_fn_pool_redeem: From::from(&deployment.const_fn_pool_redeem),
            const_fn_fee_switch_pool_swap: From::from(&deployment.const_fn_fee_switch_pool_swap),
            const_fn_fee_switch_pool_deposit: From::from(&deployment.const_fn_fee_switch_pool_deposit),
            const_fn_fee_switch_pool_redeem: From::from(&deployment.const_fn_fee_switch_pool_redeem),
            balance_fn_pool_v1: From::from(&deployment.balance_fn_pool_v1),
            balance_fn_pool_v2: From::from(&deployment.balance_fn_pool_v2),
            balance_fn_pool_deposit: From::from(&deployment.balance_fn_pool_deposit),
            balance_fn_pool_redeem: From::from(&deployment.balance_fn_pool_redeem),
            stable_fn_pool_t2t: From::from(&deployment.stable_fn_pool_t2t),
            stable_fn_pool_t2t_deposit: From::from(&deployment.stable_fn_pool_t2t_deposit),
            stable_fn_pool_t2t_redeem: From::from(&deployment.stable_fn_pool_t2t_redeem),
        }
    }
}

#[derive(Debug, Copy, Clone, Into, From)]
pub struct DeployedScriptInfo<const TYP: u8> {
    pub script_hash: ScriptHash,
    pub marginal_cost: ExUnits,
}

pub fn test_address<const TYP: u8, Ctx>(addr: &Address, ctx: &Ctx) -> bool
where
    Ctx: Has<DeployedScriptInfo<TYP>>,
{
    let maybe_hash = addr.payment_cred().and_then(|c| match c {
        StakeCredential::PubKey { .. } => None,
        StakeCredential::Script { hash, .. } => Some(hash),
    });
    if let Some(this_hash) = maybe_hash {
        return *this_hash == ctx.get().script_hash;
    }
    false
}

impl<const TYP: u8> From<&DeployedValidator<TYP>> for DeployedScriptInfo<TYP> {
    fn from(value: &DeployedValidator<TYP>) -> Self {
        Self {
            script_hash: value.hash,
            marginal_cost: value.marginal_cost,
        }
    }
}

impl<const TYP: u8> From<&DeployedValidatorRef> for DeployedScriptInfo<TYP> {
    fn from(value: &DeployedValidatorRef) -> Self {
        Self {
            script_hash: value.hash,
            marginal_cost: value.marginal_cost.unwrap_or(ExUnits::empty()),
        }
    }
}

#[derive(Debug, Clone)]
pub struct DeployedValidator<const TYP: u8> {
    pub reference_utxo: TransactionUnspentOutput,
    pub hash: ScriptHash,
    /// Cost per contract invokation.
    pub cost: ExUnits,
    /// Cost per each subsequent contract invokation.
    /// Consider a batch witness script: first invokation costs `cost`,
    /// each subsequent invokation adds `marginal_cost` to base cost.
    pub marginal_cost: ExUnits,
}

impl<const TYP: u8> DeployedValidator<TYP> {
    pub fn erased(self) -> DeployedValidatorErased {
        DeployedValidatorErased {
            reference_utxo: self.reference_utxo,
            hash: self.hash,
            ex_budget: self.cost,
            marginal_cost: self.marginal_cost,
        }
    }
}

#[derive(Debug, Clone)]
pub struct DeployedValidatorErased {
    pub reference_utxo: TransactionUnspentOutput,
    pub hash: ScriptHash,
    pub ex_budget: ExUnits,
    pub marginal_cost: ExUnits,
}

impl Display for DeployedValidatorErased {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(
            format!(
                "DeployedValidatorErased([reference_utxo: {}#{}, hash {})",
                self.reference_utxo.input.transaction_id, self.reference_utxo.input.index, self.hash
            )
            .as_str(),
        )
    }
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

impl<const TYP: u8> DeployedValidator<TYP> {
    pub async fn unsafe_pull<Net: CardanoNetwork>(v: DeployedValidatorRef, explorer: &Net) -> Self {
        let ref_output = explorer
            .utxo_by_ref(v.reference_utxo.into())
            .await
            .expect(format!("Reference UTxO {} from config not found", v.reference_utxo).as_str());
        Self {
            reference_utxo: ref_output,
            hash: v.hash,
            cost: v.cost,
            marginal_cost: v.marginal_cost.unwrap_or(ExUnits::empty()),
        }
    }
}

#[repr(u8)]
#[derive(Eq, PartialEq)]
pub enum ProtocolValidator {
    LimitOrderWitnessV1,
    LimitOrderV1,
    GridOrderNative,
    ConstFnPoolV1,
    ConstFnPoolV2,
    ConstFnPoolFeeSwitch,
    ConstFnPoolFeeSwitchV2,
    ConstFnPoolFeeSwitchBiDirFee,
    ConstFnPoolSwap,
    ConstFnPoolDeposit,
    ConstFnPoolRedeem,
    ConstFnFeeSwitchPoolSwap,
    ConstFnFeeSwitchPoolDeposit,
    ConstFnFeeSwitchPoolRedeem,
    BalanceFnPoolV1,
    BalanceFnPoolV2,
    BalanceFnPoolSwap,
    BalanceFnPoolDeposit,
    BalanceFnPoolRedeem,
    StableFnPoolT2T,
    StableFnPoolT2TDeposit,
    StableFnPoolT2TRedeem,
    DegenQuadraticPoolV1,
}

#[derive(Debug, Copy, Clone)]
pub struct ProtocolScriptHashes {
    pub limit_order_witness: DeployedScriptInfo<{ ProtocolValidator::LimitOrderWitnessV1 as u8 }>,
    pub limit_order: DeployedScriptInfo<{ ProtocolValidator::LimitOrderV1 as u8 }>,
    pub grid_order_native: DeployedScriptInfo<{ ProtocolValidator::GridOrderNative as u8 }>,
    pub const_fn_pool_v1: DeployedScriptInfo<{ ProtocolValidator::ConstFnPoolV1 as u8 }>,
    pub const_fn_pool_v2: DeployedScriptInfo<{ ProtocolValidator::ConstFnPoolV2 as u8 }>,
    pub const_fn_pool_fee_switch: DeployedScriptInfo<{ ProtocolValidator::ConstFnPoolFeeSwitch as u8 }>,
    pub const_fn_pool_fee_switch_v2: DeployedScriptInfo<{ ProtocolValidator::ConstFnPoolFeeSwitchV2 as u8 }>,
    pub const_fn_pool_fee_switch_bidir_fee:
        DeployedScriptInfo<{ ProtocolValidator::ConstFnPoolFeeSwitchBiDirFee as u8 }>,
    pub const_fn_pool_swap: DeployedScriptInfo<{ ProtocolValidator::ConstFnPoolSwap as u8 }>,
    pub const_fn_pool_deposit: DeployedScriptInfo<{ ProtocolValidator::ConstFnPoolDeposit as u8 }>,
    pub const_fn_pool_redeem: DeployedScriptInfo<{ ProtocolValidator::ConstFnPoolRedeem as u8 }>,
    pub const_fn_fee_switch_pool_swap:
        DeployedScriptInfo<{ ProtocolValidator::ConstFnFeeSwitchPoolSwap as u8 }>,
    pub const_fn_fee_switch_pool_deposit:
        DeployedScriptInfo<{ ProtocolValidator::ConstFnFeeSwitchPoolDeposit as u8 }>,
    pub const_fn_fee_switch_pool_redeem:
        DeployedScriptInfo<{ ProtocolValidator::ConstFnFeeSwitchPoolRedeem as u8 }>,
    pub balance_fn_pool_v1: DeployedScriptInfo<{ ProtocolValidator::BalanceFnPoolV1 as u8 }>,
    pub balance_fn_pool_v2: DeployedScriptInfo<{ ProtocolValidator::BalanceFnPoolV2 as u8 }>,
    pub balance_fn_pool_deposit: DeployedScriptInfo<{ ProtocolValidator::BalanceFnPoolDeposit as u8 }>,
    pub balance_fn_pool_redeem: DeployedScriptInfo<{ ProtocolValidator::BalanceFnPoolRedeem as u8 }>,
    pub stable_fn_pool_t2t: DeployedScriptInfo<{ ProtocolValidator::StableFnPoolT2T as u8 }>,
    pub stable_fn_pool_t2t_deposit: DeployedScriptInfo<{ ProtocolValidator::StableFnPoolT2TDeposit as u8 }>,
    pub stable_fn_pool_t2t_redeem: DeployedScriptInfo<{ ProtocolValidator::StableFnPoolT2TRedeem as u8 }>,
}

impl From<&ProtocolDeployment> for ProtocolScriptHashes {
    fn from(deployment: &ProtocolDeployment) -> Self {
        Self {
            limit_order_witness: From::from(&deployment.limit_order_witness),
            limit_order: From::from(&deployment.limit_order),
            grid_order_native: From::from(&deployment.grid_order_native),
            const_fn_pool_v1: From::from(&deployment.const_fn_pool_v1),
            const_fn_pool_v2: From::from(&deployment.const_fn_pool_v2),
            const_fn_pool_fee_switch: From::from(&deployment.const_fn_pool_fee_switch),
            const_fn_pool_fee_switch_v2: From::from(&deployment.const_fn_pool_fee_switch_v2),
            const_fn_pool_fee_switch_bidir_fee: From::from(&deployment.const_fn_pool_fee_switch_bidir_fee),
            const_fn_pool_swap: From::from(&deployment.const_fn_pool_swap),
            const_fn_pool_deposit: From::from(&deployment.const_fn_pool_deposit),
            const_fn_pool_redeem: From::from(&deployment.const_fn_pool_redeem),
            const_fn_fee_switch_pool_swap: From::from(&deployment.const_fn_fee_switch_pool_swap),
            const_fn_fee_switch_pool_deposit: From::from(&deployment.const_fn_fee_switch_pool_deposit),
            const_fn_fee_switch_pool_redeem: From::from(&deployment.const_fn_fee_switch_pool_redeem),
            balance_fn_pool_v1: From::from(&deployment.balance_fn_pool_v1),
            balance_fn_pool_v2: From::from(&deployment.balance_fn_pool_v2),
            balance_fn_pool_deposit: From::from(&deployment.balance_fn_pool_deposit),
            balance_fn_pool_redeem: From::from(&deployment.balance_fn_pool_redeem),
            stable_fn_pool_t2t: From::from(&deployment.stable_fn_pool_t2t),
            stable_fn_pool_t2t_deposit: From::from(&deployment.stable_fn_pool_t2t_deposit),
            stable_fn_pool_t2t_redeem: From::from(&deployment.stable_fn_pool_t2t_redeem),
        }
    }
}

#[derive(Debug, Clone)]
pub struct ProtocolDeployment {
    pub limit_order_witness: DeployedValidator<{ ProtocolValidator::LimitOrderWitnessV1 as u8 }>,
    pub limit_order: DeployedValidator<{ ProtocolValidator::LimitOrderV1 as u8 }>,
    pub grid_order_native: DeployedValidator<{ ProtocolValidator::GridOrderNative as u8 }>,
    pub const_fn_pool_v1: DeployedValidator<{ ProtocolValidator::ConstFnPoolV1 as u8 }>,
    pub const_fn_pool_v2: DeployedValidator<{ ProtocolValidator::ConstFnPoolV2 as u8 }>,
    pub const_fn_pool_fee_switch: DeployedValidator<{ ProtocolValidator::ConstFnPoolFeeSwitch as u8 }>,
    pub const_fn_pool_fee_switch_v2: DeployedValidator<{ ProtocolValidator::ConstFnPoolFeeSwitchV2 as u8 }>,
    pub const_fn_pool_fee_switch_bidir_fee:
        DeployedValidator<{ ProtocolValidator::ConstFnPoolFeeSwitchBiDirFee as u8 }>,
    pub const_fn_pool_swap: DeployedValidator<{ ProtocolValidator::ConstFnPoolSwap as u8 }>,
    pub const_fn_pool_deposit: DeployedValidator<{ ProtocolValidator::ConstFnPoolDeposit as u8 }>,
    pub const_fn_pool_redeem: DeployedValidator<{ ProtocolValidator::ConstFnPoolRedeem as u8 }>,
    pub const_fn_fee_switch_pool_swap:
        DeployedValidator<{ ProtocolValidator::ConstFnFeeSwitchPoolSwap as u8 }>,
    pub const_fn_fee_switch_pool_deposit:
        DeployedValidator<{ ProtocolValidator::ConstFnFeeSwitchPoolDeposit as u8 }>,
    pub const_fn_fee_switch_pool_redeem:
        DeployedValidator<{ ProtocolValidator::ConstFnFeeSwitchPoolRedeem as u8 }>,
    pub balance_fn_pool_v1: DeployedValidator<{ ProtocolValidator::BalanceFnPoolV1 as u8 }>,
    pub balance_fn_pool_v2: DeployedValidator<{ ProtocolValidator::BalanceFnPoolV2 as u8 }>,
    pub balance_fn_pool_deposit: DeployedValidator<{ ProtocolValidator::BalanceFnPoolDeposit as u8 }>,
    pub balance_fn_pool_redeem: DeployedValidator<{ ProtocolValidator::BalanceFnPoolRedeem as u8 }>,
    pub stable_fn_pool_t2t: DeployedValidator<{ ProtocolValidator::StableFnPoolT2T as u8 }>,
    pub stable_fn_pool_t2t_deposit: DeployedValidator<{ ProtocolValidator::StableFnPoolT2TDeposit as u8 }>,
    pub stable_fn_pool_t2t_redeem: DeployedValidator<{ ProtocolValidator::StableFnPoolT2TRedeem as u8 }>,
}

impl ProtocolDeployment {
    pub async fn unsafe_pull<Net: CardanoNetwork>(validators: DeployedValidators, explorer: &Net) -> Self {
        Self {
            limit_order_witness: DeployedValidator::unsafe_pull(validators.limit_order_witness, explorer)
                .await,
            limit_order: DeployedValidator::unsafe_pull(validators.limit_order, explorer).await,
            grid_order_native: DeployedValidator::unsafe_pull(validators.grid_order_native, explorer).await,
            const_fn_pool_v1: DeployedValidator::unsafe_pull(validators.const_fn_pool_v1, explorer).await,
            const_fn_pool_v2: DeployedValidator::unsafe_pull(validators.const_fn_pool_v2, explorer).await,
            const_fn_pool_fee_switch: DeployedValidator::unsafe_pull(
                validators.const_fn_pool_fee_switch,
                explorer,
            )
            .await,
            const_fn_pool_fee_switch_v2: DeployedValidator::unsafe_pull(
                validators.const_fn_pool_fee_switch_v2,
                explorer,
            )
            .await,
            const_fn_pool_fee_switch_bidir_fee: DeployedValidator::unsafe_pull(
                validators.const_fn_pool_fee_switch_bidir_fee,
                explorer,
            )
            .await,
            const_fn_pool_swap: DeployedValidator::unsafe_pull(validators.const_fn_pool_swap, explorer).await,
            const_fn_pool_deposit: DeployedValidator::unsafe_pull(validators.const_fn_pool_deposit, explorer)
                .await,
            const_fn_pool_redeem: DeployedValidator::unsafe_pull(validators.const_fn_pool_redeem, explorer)
                .await,
            const_fn_fee_switch_pool_swap: DeployedValidator::unsafe_pull(
                validators.const_fn_fee_switch_pool_swap,
                explorer,
            )
            .await,
            const_fn_fee_switch_pool_deposit: DeployedValidator::unsafe_pull(
                validators.const_fn_fee_switch_pool_deposit,
                explorer,
            )
            .await,
            const_fn_fee_switch_pool_redeem: DeployedValidator::unsafe_pull(
                validators.const_fn_fee_switch_pool_redeem,
                explorer,
            )
            .await,
            balance_fn_pool_v1: DeployedValidator::unsafe_pull(validators.balance_fn_pool_v1, explorer).await,
            balance_fn_pool_v2: DeployedValidator::unsafe_pull(validators.balance_fn_pool_v2, explorer).await,
            balance_fn_pool_deposit: DeployedValidator::unsafe_pull(
                validators.balance_fn_pool_deposit,
                explorer,
            )
            .await,
            balance_fn_pool_redeem: DeployedValidator::unsafe_pull(
                validators.balance_fn_pool_redeem,
                explorer,
            )
            .await,
            stable_fn_pool_t2t: DeployedValidator::unsafe_pull(validators.stable_fn_pool_t2t, explorer).await,
            stable_fn_pool_t2t_deposit: DeployedValidator::unsafe_pull(
                validators.stable_fn_pool_t2t_deposit,
                explorer,
            )
            .await,
            stable_fn_pool_t2t_redeem: DeployedValidator::unsafe_pull(
                validators.stable_fn_pool_t2t_redeem,
                explorer,
            )
            .await,
        }
    }
}

pub trait RequiresValidator<Ctx> {
    fn get_validator(&self, ctx: &Ctx) -> DeployedValidatorErased;
}
