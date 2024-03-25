use std::cmp::Ordering;

use cml_chain::builders::tx_builder::TransactionUnspentOutput;
use cml_chain::certs::Credential;
use cml_chain::plutus::{ConstrPlutusData, PlutusData};
use cml_chain::utils::BigInt;
use cml_chain::PolicyId;
use cml_core::serialization::{LenEncoding, StringEncoding};
use cml_crypto::{Ed25519KeyHash, ScriptHash};
use cml_multi_era::babbage::BabbageTransactionOutput;
use log::{info, trace};
use num_rational::Ratio;

use bloom_offchain::execution_engine::liquidity_book::fragment::{Fragment, OrderState, StateTrans};
use bloom_offchain::execution_engine::liquidity_book::side::SideM;
use bloom_offchain::execution_engine::liquidity_book::time::TimeBounds;
use bloom_offchain::execution_engine::liquidity_book::types::{
    AbsolutePrice, ExBudgetUsed, ExCostUnits, ExFeeUsed, FeeAsset, InputAsset, OutputAsset, RelativePrice,
};
use bloom_offchain::execution_engine::liquidity_book::weight::Weighted;
use spectrum_cardano_lib::address::AddressExtension;
use spectrum_cardano_lib::credential::AnyCredential;
use spectrum_cardano_lib::plutus_data::{
    ConstrPlutusDataExtension, DatumExtension, IntoPlutusData, PlutusDataExtension,
};
use spectrum_cardano_lib::transaction::TransactionOutputExtension;
use spectrum_cardano_lib::types::TryFromPData;
use spectrum_cardano_lib::value::ValueExtension;
use spectrum_cardano_lib::AssetClass;
use spectrum_offchain::data::{Has, Stable, Tradable};
use spectrum_offchain::ledger::TryFromLedger;
use spectrum_offchain_cardano::constants::SPOT_ORDER_NATIVE_TO_TOKEN_SCRIPT_HASH;
use spectrum_offchain_cardano::creds::OperatorCred;
use spectrum_offchain_cardano::data::pair::{side_of, PairId};

pub const EXEC_REDEEMER: PlutusData = PlutusData::ConstrPlutusData(ConstrPlutusData {
    alternative: 0,
    fields: vec![],
    encodings: None,
});

/// Spot order. Can be executed at a configured or better price as long as there is enough budget.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct SpotOrder {
    /// Identifier of the order.
    pub beacon: PolicyId,
    /// What user pays.
    pub input_asset: AssetClass,
    /// Remaining tradable input.
    pub input_amount: InputAsset<u64>,
    /// What user receives.
    pub output_asset: AssetClass,
    /// Accumulated output.
    pub output_amount: OutputAsset<u64>,
    /// Worst acceptable price (Output/Input).
    pub base_price: RelativePrice,
    /// Currency used to pay for execution.
    pub fee_asset: AssetClass,
    /// Remaining ADA to facilitate execution.
    pub execution_budget: FeeAsset<u64>,
    /// Fee reserved for whole swap.
    pub fee: FeeAsset<u64>,
    /// Assumed cost (in Lovelace) of one step of execution.
    pub max_cost_per_ex_step: FeeAsset<u64>,
    /// Minimal marginal output allowed per execution step.
    pub min_marginal_output: OutputAsset<u64>,
    /// Redeemer PKH.
    pub redeemer_pkh: Ed25519KeyHash,
    /// Redeemer stake PKH.
    pub redeemer_stake_cred: Option<AnyCredential>,
    /// Is executor's signature required.
    pub requires_executor_sig: bool,
}

impl SpotOrder {
    pub fn redeemer_cred(&self) -> Credential {
        Credential::PubKey {
            hash: self.redeemer_pkh,
            len_encoding: LenEncoding::Canonical,
            tag_encoding: None,
            hash_encoding: StringEncoding::Canonical,
        }
    }
}

pub fn spot_exec_redeemer(successor_ix: u16) -> PlutusData {
    PlutusData::ConstrPlutusData(ConstrPlutusData::new(
        0,
        vec![PlutusData::Integer(BigInt::from(successor_ix))],
    ))
}

impl PartialOrd for SpotOrder {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        match self.price().partial_cmp(&other.price()) {
            Some(Ordering::Equal) => self.weight().partial_cmp(&other.weight()),
            cmp => cmp,
        }
    }
}

impl Ord for SpotOrder {
    fn cmp(&self, other: &Self) -> Ordering {
        match self.price().cmp(&other.price()) {
            Ordering::Equal => self.weight().cmp(&other.weight()),
            cmp => cmp,
        }
    }
}

impl OrderState for SpotOrder {
    fn with_updated_time(self, _time: u64) -> StateTrans<Self> {
        StateTrans::Active(self)
    }

    fn with_applied_swap(
        mut self,
        removed_input: u64,
        added_output: u64,
    ) -> (StateTrans<Self>, ExBudgetUsed, ExFeeUsed) {
        self.input_amount -= removed_input;
        self.output_amount += added_output;
        let budget_used = self.max_cost_per_ex_step;
        self.execution_budget -= budget_used;
        let fee_used = self.linear_fee(removed_input);
        self.fee -= fee_used;
        let next_st = if self.execution_budget < self.max_cost_per_ex_step || self.input_amount == 0 {
            StateTrans::EOL
        } else {
            StateTrans::Active(self)
        };
        (next_st, budget_used, fee_used)
    }
}

impl Fragment for SpotOrder {
    fn side(&self) -> SideM {
        side_of(self.input_asset, self.output_asset)
    }

    fn input(&self) -> u64 {
        self.input_amount
    }

    fn price(&self) -> AbsolutePrice {
        AbsolutePrice::from_price(self.side(), self.base_price)
    }

    fn linear_fee(&self, input_consumed: InputAsset<u64>) -> FeeAsset<u64> {
        if self.input_amount > 0 {
            self.fee * input_consumed / self.input_amount
        } else {
            0
        }
    }

    fn weighted_fee(&self) -> FeeAsset<Ratio<u64>> {
        Ratio::new(self.fee, self.input_amount)
    }

    fn marginal_cost_hint(&self) -> ExCostUnits {
        160000000
    }

    fn time_bounds(&self) -> TimeBounds<u64> {
        TimeBounds::None
    }
}

impl Stable for SpotOrder {
    type StableId = PolicyId;
    fn stable_id(&self) -> Self::StableId {
        self.beacon
    }
    fn is_quasi_permanent(&self) -> bool {
        false
    }
}

impl Tradable for SpotOrder {
    type PairId = PairId;

    fn pair_id(&self) -> Self::PairId {
        PairId::canonical(self.input_asset, self.output_asset)
    }
}

struct ConfigNativeToToken {
    pub beacon: PolicyId,
    pub tradable_input: InputAsset<u64>,
    pub cost_per_ex_step: FeeAsset<u64>,
    pub min_marginal_output: OutputAsset<u64>,
    pub output: AssetClass,
    pub base_price: RelativePrice,
    pub fee: FeeAsset<u64>,
    pub redeemer_pkh: Ed25519KeyHash,
    pub permitted_executors: Vec<Ed25519KeyHash>,
}

struct DatumNativeToTokenMapping {
    pub beacon: usize,
    pub tradable_input: usize,
    pub cost_per_ex_step: usize,
    pub min_marginal_output: usize,
    pub output: usize,
    pub base_price: usize,
    pub fee: usize,
    pub redeemer_pkh: usize,
    pub permitted_executors: usize,
}

pub const N2T_DATUM_MAPPING: DatumNativeToTokenMapping = DatumNativeToTokenMapping {
    beacon: 0,
    tradable_input: 1,
    cost_per_ex_step: 2,
    min_marginal_output: 3,
    output: 4,
    base_price: 5,
    fee: 6,
    redeemer_pkh: 7,
    permitted_executors: 8,
};

pub fn unsafe_update_n2t_variables(
    data: &mut PlutusData,
    tradable_input: InputAsset<u64>,
    fee: FeeAsset<u64>,
) {
    let cpd = data.get_constr_pd_mut().unwrap();
    cpd.set_field(N2T_DATUM_MAPPING.tradable_input, tradable_input.into_pd());
    cpd.set_field(N2T_DATUM_MAPPING.fee, fee.into_pd());
}

impl TryFromPData for ConfigNativeToToken {
    fn try_from_pd(data: PlutusData) -> Option<Self> {
        let mut cpd = data.into_constr_pd()?;
        let beacon = <[u8; 28]>::try_from(cpd.take_field(N2T_DATUM_MAPPING.beacon)?.into_bytes()?)
            .map(PolicyId::from)
            .ok()?;
        let tradable_input = cpd.take_field(N2T_DATUM_MAPPING.tradable_input)?.into_u64()?;
        let cost_per_ex_step = cpd.take_field(N2T_DATUM_MAPPING.cost_per_ex_step)?.into_u64()?;
        let min_marginal_output = cpd
            .take_field(N2T_DATUM_MAPPING.min_marginal_output)?
            .into_u64()?;
        let output = AssetClass::try_from_pd(cpd.take_field(N2T_DATUM_MAPPING.output)?)?;
        let base_price = RelativePrice::try_from_pd(cpd.take_field(N2T_DATUM_MAPPING.base_price)?)?;
        let fee = cpd.take_field(N2T_DATUM_MAPPING.fee)?.into_u64()?;
        let redeemer_pkh = Ed25519KeyHash::from(
            <[u8; 28]>::try_from(cpd.take_field(N2T_DATUM_MAPPING.redeemer_pkh)?.into_bytes()?).ok()?,
        );
        let permitted_executors = cpd
            .take_field(N2T_DATUM_MAPPING.permitted_executors)?
            .into_vec()?
            .into_iter()
            .filter_map(|pd| Some(Ed25519KeyHash::from(<[u8; 28]>::try_from(pd.into_bytes()?).ok()?)))
            .collect();
        Some(ConfigNativeToToken {
            beacon,
            tradable_input,
            cost_per_ex_step,
            min_marginal_output,
            output,
            base_price,
            fee,
            redeemer_pkh,
            permitted_executors,
        })
    }
}

impl<C> TryFromLedger<BabbageTransactionOutput, C> for SpotOrder
where
    C: Has<OperatorCred>,
{
    fn try_from_ledger(repr: &BabbageTransactionOutput, ctx: &C) -> Option<Self> {
        let script_hash = ScriptHash::from_hex(SPOT_ORDER_NATIVE_TO_TOKEN_SCRIPT_HASH).unwrap();
        trace!(target: "offchain", "SpotOrder::try_from_ledger");
        if repr.address().script_hash() == Some(script_hash) {
            info!(target: "offchain", "Spot order address coincides");
            let value = repr.value().clone();
            let conf = ConfigNativeToToken::try_from_pd(repr.datum()?.into_pd()?)?;
            let total_ada_input = value.amount_of(AssetClass::Native)?;
            if total_ada_input < conf.tradable_input {
                return None;
            }
            let execution_budget = total_ada_input - conf.tradable_input;
            let is_permissionless = conf.permitted_executors.is_empty();
            if is_permissionless || conf.permitted_executors.contains(&ctx.get().into()) {
                if execution_budget > conf.cost_per_ex_step {
                    info!(target: "offchain", "Obtained Spot order from ledger.");
                    return Some(SpotOrder {
                        beacon: conf.beacon,
                        input_asset: AssetClass::Native,
                        input_amount: conf.tradable_input,
                        output_asset: conf.output,
                        output_amount: value.amount_of(conf.output).unwrap_or(0),
                        base_price: conf.base_price,
                        execution_budget,
                        fee_asset: AssetClass::Native,
                        fee: conf.fee,
                        min_marginal_output: conf.min_marginal_output,
                        max_cost_per_ex_step: conf.cost_per_ex_step,
                        redeemer_pkh: conf.redeemer_pkh,
                        redeemer_stake_cred: repr.address().staking_cred().cloned().map(|x| x.into()),
                        requires_executor_sig: !is_permissionless,
                    });
                }
            }
        }

        None
    }
}

/// Reference Script Output for [SpotOrder].
#[derive(Debug, Clone)]
pub struct SpotOrderRefScriptOutput(pub TransactionUnspentOutput);

/// Reference Script Output for batch validator of [SpotOrder].
#[derive(Debug, Clone)]
pub struct SpotOrderBatchValidatorRefScriptOutput(pub TransactionUnspentOutput);
