use crate::orders::limit::{LimitOrder, LimitOrderBounds};
use bloom_offchain::execution_engine::liquidity_book::core::{Next, TerminalTake, Unit};
use bloom_offchain::execution_engine::liquidity_book::market_taker::{MarketTaker, TakerBehaviour};
use bloom_offchain::execution_engine::liquidity_book::side::Side;
use bloom_offchain::execution_engine::liquidity_book::time::TimeBounds;
use bloom_offchain::execution_engine::liquidity_book::types::{
    AbsolutePrice, FeeAsset, InputAsset, Lovelace, OutputAsset,
};
use bounded_integer::BoundedU64;
use cml_multi_era::babbage::BabbageTransactionOutput;
use spectrum_cardano_lib::ex_units::ExUnits;
use spectrum_cardano_lib::AssetClass;
use spectrum_offchain::data::{Has, Stable, Tradable};
use spectrum_offchain::ledger::TryFromLedger;
use spectrum_offchain_cardano::creds::OperatorCred;
use spectrum_offchain_cardano::deployment::DeployedScriptInfo;
use spectrum_offchain_cardano::deployment::ProtocolValidator::LimitOrderV1;
use spectrum_offchain_cardano::utxo::ConsumedInputs;
use std::cmp::Ordering;
use std::fmt::{Display, Formatter};

#[derive(Copy, Clone, Debug)]
pub struct AdhocFeeStructure {
    pub fixed_fee_lovelace: Lovelace,
    pub relative_fee_percent: BoundedU64<0, 100>,
}

impl AdhocFeeStructure {
    pub fn empty() -> Self {
        Self {
            fixed_fee_lovelace: 0,
            relative_fee_percent: BoundedU64::new_saturating(0),
        }
    }
}

/// A version of [LimitOrder] with ad-hoc fee algorithm.
/// Fee is charged as % of the trade from the side that contains ADA.
#[derive(Debug, Copy, Clone)]
pub struct AdhocOrder(pub(crate) LimitOrder);

impl Display for AdhocOrder {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(format!("AdhocOrder({})", self.0).as_str())
    }
}

impl PartialEq for AdhocOrder {
    fn eq(&self, other: &Self) -> bool {
        self.0.eq(&other.0)
    }
}

impl Eq for AdhocOrder {}

impl PartialOrd for AdhocOrder {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.0.cmp(&other.0))
    }
}

impl Ord for AdhocOrder {
    fn cmp(&self, other: &Self) -> Ordering {
        self.0.cmp(&other.0)
    }
}

impl TakerBehaviour for AdhocOrder {
    fn with_updated_time(self, _: u64) -> Next<Self, Unit> {
        Next::Succ(self)
    }

    fn with_applied_trade(
        mut self,
        removed_input: InputAsset<u64>,
        added_output: OutputAsset<u64>,
    ) -> Next<Self, TerminalTake> {
        self.0
            .with_applied_trade(removed_input, added_output)
            .map_succ(AdhocOrder)
    }

    fn with_budget_corrected(mut self, delta: i64) -> (i64, Self) {
        let (real_delta, order) = self.0.with_budget_corrected(delta);
        (real_delta, AdhocOrder(order))
    }

    fn with_fee_charged(mut self, fee: u64) -> Self {
        AdhocOrder(self.0.with_fee_charged(fee))
    }

    fn with_output_added(mut self, added_output: u64) -> Self {
        AdhocOrder(self.0.with_output_added(added_output))
    }

    fn try_terminate(self) -> Next<Self, TerminalTake> {
        self.0.try_terminate().map_succ(AdhocOrder)
    }
}

impl MarketTaker for AdhocOrder {
    type U = ExUnits;

    fn side(&self) -> Side {
        self.0.side()
    }

    fn input(&self) -> u64 {
        let original_input = self.0.input();
        match self.side() {
            Side::Bid => original_input,
            Side::Ask => original_input,
        }
    }

    fn output(&self) -> OutputAsset<u64> {
        self.0.output()
    }

    fn price(&self) -> AbsolutePrice {
        self.0.price()
    }

    fn operator_fee(&self, input_consumed: InputAsset<u64>) -> FeeAsset<u64> {
        self.0.operator_fee(input_consumed)
    }

    fn fee(&self) -> FeeAsset<u64> {
        self.0.fee()
    }

    fn budget(&self) -> FeeAsset<u64> {
        self.0.budget()
    }

    fn consumable_budget(&self) -> FeeAsset<u64> {
        self.0.consumable_budget()
    }

    fn marginal_cost_hint(&self) -> ExUnits {
        self.0.marginal_cost_hint()
    }

    fn min_marginal_output(&self) -> OutputAsset<u64> {
        self.0.min_marginal_output()
    }

    fn time_bounds(&self) -> TimeBounds<u64> {
        self.0.time_bounds()
    }
}

impl Stable for AdhocOrder {
    type StableId = <LimitOrder as Stable>::StableId;
    fn stable_id(&self) -> Self::StableId {
        self.0.stable_id()
    }
    fn is_quasi_permanent(&self) -> bool {
        self.0.is_quasi_permanent()
    }
}

impl Tradable for AdhocOrder {
    type PairId = <LimitOrder as Tradable>::PairId;

    fn pair_id(&self) -> Self::PairId {
        self.0.pair_id()
    }
}

pub(crate) fn subtract_adhoc_fee(target: u64, fee_structure: AdhocFeeStructure) -> Option<u64> {
    let body_without_fixed_fee = target.checked_sub(fee_structure.fixed_fee_lovelace)?;
    let relative_fee = body_without_fixed_fee * fee_structure.relative_fee_percent.get() / 100;
    Some(body_without_fixed_fee.checked_sub(relative_fee)?)
}

impl<C> TryFromLedger<BabbageTransactionOutput, C> for AdhocOrder
where
    C: Has<OperatorCred>
        + Has<ConsumedInputs>
        + Has<DeployedScriptInfo<{ LimitOrderV1 as u8 }>>
        + Has<LimitOrderBounds>
        + Has<AdhocFeeStructure>,
{
    fn try_from_ledger(repr: &BabbageTransactionOutput, ctx: &C) -> Option<Self> {
        LimitOrder::try_from_ledger(repr, ctx).and_then(|lo| {
            let virtual_input_amount = match (lo.input_asset, lo.output_asset) {
                (AssetClass::Native, _) => subtract_adhoc_fee(lo.input_amount, ctx.get()),
                (_, AssetClass::Native) => Some(lo.input_amount),
                _ => None,
            };
            Some(Self(LimitOrder {
                beacon: lo.beacon,
                input_asset: lo.input_asset,
                input_amount: virtual_input_amount?,
                output_asset: lo.output_asset,
                output_amount: lo.output_amount,
                base_price: lo.base_price,
                fee_asset: lo.fee_asset,
                execution_budget: lo.execution_budget,
                fee: lo.fee,
                max_cost_per_ex_step: lo.max_cost_per_ex_step,
                min_marginal_output: lo.min_marginal_output,
                redeemer_address: lo.redeemer_address,
                cancellation_pkh: lo.cancellation_pkh,
                requires_executor_sig: lo.requires_executor_sig,
                virgin: lo.virgin,
                marginal_cost: lo.marginal_cost,
            }))
        })
    }
}
