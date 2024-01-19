use std::cmp::Ordering;

use cml_chain::certs::Credential;
use cml_chain::plutus::{ConstrPlutusData, PlutusData};
use cml_chain::utils::BigInt;
use cml_chain::PolicyId;
use cml_core::serialization::{LenEncoding, StringEncoding};
use cml_crypto::Ed25519KeyHash;
use cml_multi_era::babbage::BabbageTransactionOutput;

use crate::creds::ExecutorCred;
use bloom_offchain::execution_engine::liquidity_book::fragment::{
    ExBudgetUsed, Fragment, OrderState, StateTrans,
};
use bloom_offchain::execution_engine::liquidity_book::side::SideM;
use bloom_offchain::execution_engine::liquidity_book::time::TimeBounds;
use bloom_offchain::execution_engine::liquidity_book::types::{
    AbsolutePrice, ExCostUnits, FeeExtension, FeePerOutput, RelativePrice,
};
use bloom_offchain::execution_engine::liquidity_book::weight::Weighted;
use spectrum_cardano_lib::credential::AnyCredential;
use spectrum_cardano_lib::plutus_data::{ConstrPlutusDataExtension, DatumExtension, PlutusDataExtension};
use spectrum_cardano_lib::transaction::TransactionOutputExtension;
use spectrum_cardano_lib::types::TryFromPData;
use spectrum_cardano_lib::value::ValueExtension;
use spectrum_cardano_lib::AssetClass;
use spectrum_offchain::data::{Has, Stable, Tradable};
use spectrum_offchain::ledger::TryFromLedger;
use spectrum_offchain_cardano::data::pair::{side_of, PairId};

pub type FeeCurrency<T> = T;

/// Spot order. Can be executed at a configured or better price as long as there is enough budget.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct SpotOrder {
    /// Identifier of the order.
    pub beacon: PolicyId,
    /// What user pays.
    pub input_asset: AssetClass,
    /// Remaining tradable input.
    pub input_amount: u64,
    /// What user receives.
    pub output_asset: AssetClass,
    /// Accumulated output.
    pub output_amount: u64,
    /// Worst acceptable price (Output/Input).
    pub base_price: RelativePrice,
    /// Currency used to pay for execution.
    pub fee_asset: AssetClass,
    /// Remaining ADA to facilitate execution.
    pub execution_budget: FeeCurrency<u64>,
    /// Fee per output unit.
    pub fee_per_output: FeeCurrency<FeePerOutput>,
    /// Assumed cost (in Lovelace) of one step of execution.
    pub max_cost_per_ex_step: FeeCurrency<u64>,
    /// Minimal marginal output allowed per execution step.
    pub min_marginal_output: u64,
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
    ) -> (StateTrans<Self>, ExBudgetUsed) {
        self.input_amount -= removed_input;
        self.output_amount += added_output;
        let budget_used = self.fee_per_output.linear_fee(added_output) + self.max_cost_per_ex_step;
        self.execution_budget -= budget_used;
        let next_st = if self.execution_budget < self.max_cost_per_ex_step || self.input_amount == 0 {
            StateTrans::EOL
        } else {
            StateTrans::Active(self)
        };
        (next_st, budget_used)
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

    fn fee(&self) -> FeePerOutput {
        self.fee_per_output
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
}

impl Tradable for SpotOrder {
    type PairId = PairId;

    fn pair_id(&self) -> Self::PairId {
        PairId::canonical(self.input_asset, self.output_asset)
    }
}

struct ConfigNativeToToken {
    pub beacon: PolicyId,
    pub tradable_input: u64,
    pub cost_per_ex_step: u64,
    pub min_marginal_output: u64,
    pub output: AssetClass,
    pub base_price: RelativePrice,
    pub fee_per_output: FeePerOutput,
    pub redeemer_pkh: Ed25519KeyHash,
    pub permitted_executors: Vec<Ed25519KeyHash>,
}

impl TryFromPData for ConfigNativeToToken {
    fn try_from_pd(data: PlutusData) -> Option<Self> {
        let mut cpd = data.into_constr_pd()?;
        Some(ConfigNativeToToken {
            beacon: <[u8; 28]>::try_from(cpd.take_field(0)?.into_bytes()?)
                .map(PolicyId::from)
                .ok()?,
            tradable_input: cpd.take_field(1)?.into_u64()?,
            cost_per_ex_step: cpd.take_field(2)?.into_u64()?,
            min_marginal_output: cpd.take_field(3)?.into_u64()?,
            output: AssetClass::try_from_pd(cpd.take_field(4)?)?,
            base_price: RelativePrice::try_from_pd(cpd.take_field(5)?)?,
            fee_per_output: FeePerOutput::try_from_pd(cpd.take_field(6)?)?,
            redeemer_pkh: Ed25519KeyHash::from(<[u8; 28]>::try_from(cpd.take_field(7)?.into_bytes()?).ok()?),
            permitted_executors: cpd
                .take_field(8)?
                .into_vec()?
                .into_iter()
                .filter_map(|pd| Some(Ed25519KeyHash::from(<[u8; 28]>::try_from(pd.into_bytes()?).ok()?)))
                .collect(),
        })
    }
}

impl<C> TryFromLedger<BabbageTransactionOutput, C> for SpotOrder
where
    C: Has<ExecutorCred>,
{
    fn try_from_ledger(repr: &BabbageTransactionOutput, ctx: C) -> Option<Self> {
        let value = repr.value().clone();
        let conf = ConfigNativeToToken::try_from_pd(repr.datum()?.into_pd()?)?;
        let total_ada_input = value.amount_of(AssetClass::Native)?;
        let execution_budget = total_ada_input - conf.tradable_input;
        let is_permissionless = conf.permitted_executors.is_empty();
        if is_permissionless || conf.permitted_executors.contains(&ctx.get().into()) {
            if execution_budget > conf.cost_per_ex_step {
                return Some(SpotOrder {
                    beacon: conf.beacon,
                    input_asset: AssetClass::Native,
                    input_amount: conf.tradable_input,
                    output_asset: conf.output,
                    output_amount: value.amount_of(conf.output)?,
                    base_price: conf.base_price,
                    execution_budget,
                    fee_asset: AssetClass::Native,
                    fee_per_output: conf.fee_per_output,
                    min_marginal_output: conf.min_marginal_output,
                    max_cost_per_ex_step: conf.cost_per_ex_step,
                    redeemer_pkh: conf.redeemer_pkh,
                    redeemer_stake_cred: repr.address().staking_cred().cloned().map(|x| x.into()),
                    requires_executor_sig: !is_permissionless,
                });
            }
        }
        None
    }
}
