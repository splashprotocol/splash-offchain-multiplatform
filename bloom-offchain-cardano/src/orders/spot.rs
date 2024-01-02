use std::cmp::Ordering;

use cml_chain::plutus::{ConstrPlutusData, PlutusData};
use cml_chain::utils::BigInt;
use cml_chain::PolicyId;
use cml_crypto::Ed25519KeyHash;
use cml_multi_era::babbage::BabbageTransactionOutput;
use num_rational::Ratio;

use bloom_offchain::execution_engine::liquidity_book::fragment::{Fragment, OrderState, StateTrans};
use bloom_offchain::execution_engine::liquidity_book::side::SideM;
use bloom_offchain::execution_engine::liquidity_book::time::TimeBounds;
use bloom_offchain::execution_engine::liquidity_book::types::{AbsolutePrice, ExecutionCost, RelativePrice};
use spectrum_cardano_lib::{AssetClass, OutputRef};
use spectrum_offchain::data::{Stable, Tradable};
use spectrum_offchain::ledger::TryFromLedger;
use spectrum_offchain_cardano::data::ExecutorFeePerToken;

use crate::{side_of, PairId};

/// Spot order. Can be executed at a configured or better price as long as there is enough budget.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct SpotOrder {
    /// Identifier of the order.
    pub beacon: PolicyId,
    /// What user pays.
    pub input_asset: AssetClass,
    /// Remaining input.
    pub input_amount: u64,
    /// What user receives.
    pub output_asset: AssetClass,
    /// Accumulated output.
    pub output_amount: u64,
    /// Amount of output sufficient for the order to be terminated.
    pub termination_threshold: u64,
    /// Worst acceptable price (Output/Input).
    pub base_price: RelativePrice,
    /// Fee per output unit.
    pub fee_per_output: ExecutorFeePerToken,
    /// Redeemer PKH.
    pub redeemer_pkh: Ed25519KeyHash,
    /// Redeemer stake PKH.
    pub redeemer_stake_pkh: Option<Ed25519KeyHash>,
}

fn spot_exec_redeemer(successor_ix: u16) -> PlutusData {
    PlutusData::ConstrPlutusData(ConstrPlutusData::new(
        0,
        vec![PlutusData::Integer(BigInt::from(successor_ix))],
    ))
}

impl PartialOrd for SpotOrder {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        self.weight().partial_cmp(&other.weight())
    }
}

impl Ord for SpotOrder {
    fn cmp(&self, other: &Self) -> Ordering {
        self.weight().cmp(&other.weight())
    }
}

impl OrderState for SpotOrder {
    fn with_updated_time(self, _time: u64) -> StateTrans<Self> {
        StateTrans::Active(self)
    }

    fn with_updated_liquidity(mut self, removed_input: u64, added_output: u64) -> StateTrans<Self> {
        self.input_amount -= removed_input;
        self.output_amount += added_output;
        if self.output_amount >= self.termination_threshold || self.input_amount == 0 {
            StateTrans::EOL
        } else {
            StateTrans::Active(self)
        }
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

    fn weight(&self) -> Ratio<u128> {
        self.fee_per_output.value()
    }

    fn cost_hint(&self) -> ExecutionCost {
        1 // todo
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

impl TryFromLedger<BabbageTransactionOutput, OutputRef> for SpotOrder {
    fn try_from_ledger(repr: &BabbageTransactionOutput, ctx: OutputRef) -> Option<Self> {
        todo!()
    }
}
