use crate::orders::instant::{InstantOrder, InstantOrderValidation};
use crate::orders::limit::{LimitOrder, LimitOrderValidation};
use bloom_offchain::execution_engine::liquidity_book::core::{Next, TerminalTake, Unit};
use bloom_offchain::execution_engine::liquidity_book::market_taker::{MarketTaker, TakerBehaviour};
use bloom_offchain::execution_engine::liquidity_book::side::Side;
use bloom_offchain::execution_engine::liquidity_book::time::TimeBounds;
use bloom_offchain::execution_engine::liquidity_book::types::{
    AbsolutePrice, FeeAsset, InputAsset, Lovelace, OutputAsset, RelativePrice,
};
use bounded_integer::BoundedU64;
use cml_chain::transaction::TransactionOutput;
use log::trace;
use spectrum_cardano_lib::ex_units::ExUnits;
use spectrum_cardano_lib::{AssetClass, OutputRef, Token};
use spectrum_offchain::domain::{Has, SeqState, Stable, Tradable};
use spectrum_offchain::ledger::TryFromLedger;
use spectrum_offchain_cardano::creds::OperatorCred;
use spectrum_offchain_cardano::deployment::DeployedScriptInfo;
use spectrum_offchain_cardano::deployment::ProtocolValidator::InstantOrderV1;
use spectrum_offchain_cardano::handler_context::{
    AddedPaymentDestinations, AllowedAdditionalPaymentDestinations, ConsumedIdentifiers, ConsumedInputs,
    ProducedIdentifiers,
};
use std::cmp::Ordering;
use std::fmt::{Display, Formatter};

#[derive(Copy, Clone, Debug)]
pub struct AdhocFeeStructure {
    pub relative_fee_percent: BoundedU64<0, 100>,
}

impl AdhocFeeStructure {
    pub fn empty() -> Self {
        Self {
            relative_fee_percent: BoundedU64::new_saturating(0),
        }
    }

    pub fn fee(&self, body: u64) -> u64 {
        body * self.relative_fee_percent.get() / 100
    }
}

/// A version of [LimitOrder] with ad-hoc fee algorithm.
/// Fee is charged as % of the trade from the side that contains ADA.
#[derive(Debug, Copy, Clone)]
pub struct AdhocOrder(pub InstantOrder, /*adhoc_fee_input*/ pub(crate) u64);

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
        self,
        removed_input: InputAsset<u64>,
        added_output: OutputAsset<u64>,
    ) -> Next<Self, TerminalTake> {
        self.0
            .with_applied_trade(removed_input, added_output)
            .map_succ(|s| AdhocOrder(s, self.1))
    }

    fn with_budget_corrected(self, delta: i64) -> (i64, Self) {
        let (real_delta, order) = self.0.with_budget_corrected(delta);
        (real_delta, AdhocOrder(order, self.1))
    }

    fn with_fee_charged(self, fee: u64) -> Self {
        AdhocOrder(self.0.with_fee_charged(fee), self.1)
    }

    fn with_output_added(self, added_output: u64) -> Self {
        AdhocOrder(self.0.with_output_added(added_output), self.1)
    }

    fn try_terminate(self) -> Next<Self, TerminalTake> {
        self.0.try_terminate().map_succ(|s| AdhocOrder(s, self.1))
    }
}

impl MarketTaker for AdhocOrder {
    type U = ExUnits;

    fn side(&self) -> Side {
        self.0.side()
    }

    fn input(&self) -> u64 {
        self.0.input()
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
    type StableId = <InstantOrder as Stable>::StableId;
    fn stable_id(&self) -> Self::StableId {
        self.0.stable_id()
    }
    fn is_quasi_permanent(&self) -> bool {
        self.0.is_quasi_permanent()
    }
}

impl SeqState for AdhocOrder {
    fn is_initial(&self) -> bool {
        self.0.is_initial()
    }
}

impl Tradable for AdhocOrder {
    type PairId = <LimitOrder as Tradable>::PairId;

    fn pair_id(&self) -> Self::PairId {
        self.0.pair_id()
    }
}

fn subtract_adhoc_fee(body: u64, fee_structure: AdhocFeeStructure) -> u64 {
    body - fee_structure.fee(body)
}

impl<C> TryFromLedger<TransactionOutput, C> for AdhocOrder
where
    C: Has<OperatorCred>
        + Has<OutputRef>
        + Has<ConsumedIdentifiers<Token>>
        + Has<ProducedIdentifiers<Token>>
        + Has<ConsumedInputs>
        + Has<AddedPaymentDestinations>
        + Has<AllowedAdditionalPaymentDestinations>
        + Has<DeployedScriptInfo<{ InstantOrderV1 as u8 }>>
        + Has<InstantOrderValidation>
        + Has<AdhocFeeStructure>,
{
    fn try_from_ledger(repr: &TransactionOutput, ctx: &C) -> Option<Self> {
        InstantOrder::try_from_ledger(repr, ctx).and_then(|io| {
            let virtual_input_amount = match (io.input_asset, io.output_asset) {
                (AssetClass::Native, _) => subtract_adhoc_fee(io.input_amount, ctx.get()),
                _ => io.input_amount,
            };
            // adhoc fee input is only applicable in case of ADA -> TOKEN swap
            let adhoc_fee_input = if io.input_asset == AssetClass::Native {
                io.input_amount.checked_sub(virtual_input_amount)?
            } else {
                0
            };
            let has_stake_part = io.redeemer_address.stake_cred.is_some();
            let is_compliant = ctx
                .select::<AddedPaymentDestinations>()
                .complies_with(&ctx.select::<AllowedAdditionalPaymentDestinations>());
            if has_stake_part && is_compliant {
                Some(Self(
                    InstantOrder {
                        input_amount: virtual_input_amount,
                        ..io
                    },
                    adhoc_fee_input,
                ))
            } else {
                trace!(
                    "AdhocOrder skipped for UTxO {}, AdhocOrder {} :: has_stake_part: {}, is_compliant: {}",
                    ctx.select::<OutputRef>(),
                    io.beacon,
                    has_stake_part,
                    is_compliant
                );
                None
            }
        })
    }
}
