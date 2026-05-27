use std::cmp::{max, Ordering};
use std::collections::HashMap;
use std::fmt::{Display, Formatter};

use algebra_core::monoid::Monoid;
use bloom_offchain::execution_engine::liquidity_book::core::{Next, TerminalTake, Unit};
use bloom_offchain::execution_engine::liquidity_book::market_taker::{MarketTaker, TakerBehaviour};
use bloom_offchain::execution_engine::liquidity_book::side::Side;
use bloom_offchain::execution_engine::liquidity_book::time::TimeBounds;
use bloom_offchain::execution_engine::liquidity_book::types::{
    AbsolutePrice, FeeAsset, InputAsset, OutputAsset, RelativePrice,
};
use bloom_offchain::execution_engine::liquidity_book::weight::Weighted;
use bloom_offchain::execution_engine::types::Time;
use cml_chain::plutus::{ConstrPlutusData, PlutusData};
use cml_chain::transaction::TransactionOutput;
use cml_chain::PolicyId;
use cml_core::serialization::Serialize;
use cml_crypto::{blake2b224, Ed25519KeyHash, RawBytesEncoding, ScriptHash};
use num_rational::Ratio;
use spectrum_cardano_lib::plutus_data::{
    ConstrPlutusDataExtension, DatumExtension, IntoPlutusData, PlutusDataExtension,
};
use spectrum_cardano_lib::transaction::TransactionOutputExtension;
use spectrum_cardano_lib::types::TryFromPData;
use spectrum_cardano_lib::value::ValueExtension;
use spectrum_cardano_lib::{AssetClass, AssetName, OutputRef, Token};
use spectrum_offchain::domain::{Has, SeqState, Stable, Tradable};
use spectrum_offchain::ledger::TryFromLedger;
use spectrum_offchain_cardano::data::pair::{side_of, PairId};
use spectrum_offchain_cardano::deployment::DeployedValidatorErased;

const PRICE_DECAY_DENOM: u128 = 1000;

#[derive(Debug, Copy, Clone, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuctionOrderConfig {
    pub base_asset: AssetClass,
    pub quote_asset: AssetClass,
    pub price_start: RelativePrice,
    pub start_time: u64,
    pub step_len: u64,
    pub steps: u64,
    pub price_decay_num: u128,
    pub fee_per_quote: RelativePrice,
    pub redeemer: Ed25519KeyHash,
}

impl AuctionOrderConfig {
    pub fn span_at_time(&self, time: u64) -> Option<u64> {
        if self.step_len == 0 || time < self.start_time {
            return None;
        }
        let span = (time - self.start_time) / self.step_len;
        (span < self.steps).then_some(span)
    }

    pub fn next_activation_time(&self, time: u64) -> Option<u64> {
        if time < self.start_time {
            Some(self.start_time)
        } else {
            self.span_at_time(time).map(|span| {
                self.start_time
                    .saturating_add(self.step_len.saturating_mul(span.saturating_add(1)))
            })
        }
    }

    pub fn end_time(&self) -> u64 {
        self.start_time
            .saturating_add(self.step_len.saturating_mul(self.steps))
    }

    pub fn price_at_span(&self, span: u64) -> RelativePrice {
        let mut num = *self.price_start.numer();
        let mut denom = *self.price_start.denom();
        for _ in 0..span {
            num = num.saturating_mul(self.price_decay_num);
            denom = denom.saturating_mul(PRICE_DECAY_DENOM);
        }
        Ratio::new_raw(num, denom)
    }

    #[cfg(test)]
    pub fn default_for_tests() -> Self {
        Self {
            base_asset: AssetClass::Native,
            quote_asset: AssetClass::Native,
            price_start: RelativePrice::new_raw(1, 1),
            start_time: 100,
            step_len: 10,
            steps: 5,
            price_decay_num: 1000,
            fee_per_quote: RelativePrice::new_raw(0, 1),
            redeemer: Ed25519KeyHash::from_raw_bytes(&[0u8; 28]).unwrap(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct AuctionOrderRegistryEntry {
    pub validator: DeployedValidatorErased,
    pub max_cost_per_ex_step: FeeAsset<u64>,
    pub min_marginal_output: OutputAsset<u64>,
}

#[derive(Clone, Debug, Default)]
pub struct AuctionOrderRegistry {
    entries: HashMap<ScriptHash, AuctionOrderRegistryEntry>,
}

impl AuctionOrderRegistry {
    pub fn new(entries: impl IntoIterator<Item = AuctionOrderRegistryEntry>) -> Self {
        Self {
            entries: entries
                .into_iter()
                .map(|entry| (entry.validator.hash, entry))
                .collect(),
        }
    }

    pub fn get(&self, script_hash: &ScriptHash) -> Option<&AuctionOrderRegistryEntry> {
        self.entries.get(script_hash)
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct AuctionOrder {
    pub id: PolicyId,
    pub script_hash: ScriptHash,
    pub config: AuctionOrderConfig,
    pub input_amount: InputAsset<u64>,
    pub output_amount: OutputAsset<u64>,
    pub execution_budget: FeeAsset<u64>,
    pub fee: FeeAsset<u64>,
    pub max_cost_per_ex_step: FeeAsset<u64>,
    pub min_marginal_output: OutputAsset<u64>,
    pub current_span: u64,
}

impl AuctionOrder {
    fn active_price(&self) -> RelativePrice {
        self.config.price_at_span(self.current_span)
    }

    fn fee_for_input(&self, input_consumed: u64) -> u64 {
        let quote = input_consumed as u128 * *self.active_price().numer() / *self.active_price().denom();
        (quote * *self.config.fee_per_quote.numer() / *self.config.fee_per_quote.denom()) as u64
    }
}

impl Display for AuctionOrder {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(
            format!(
                "AuctionOrder({}, {}, {}, span={}, p={}, in={} {}, out={} {}, budget={})",
                self.script_hash,
                self.side(),
                self.pair_id(),
                self.current_span,
                self.price(),
                self.input_amount,
                self.config.base_asset,
                self.output_amount,
                self.config.quote_asset,
                self.execution_budget,
            )
            .as_str(),
        )
    }
}

impl PartialOrd for AuctionOrder {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for AuctionOrder {
    fn cmp(&self, other: &Self) -> Ordering {
        let cmp_by_price = self.price().cmp(&other.price());
        let cmp_by_price = if matches!(self.side(), Side::Bid) {
            cmp_by_price.reverse()
        } else {
            cmp_by_price
        };
        cmp_by_price
            .then(self.weight().cmp(&other.weight()))
            .then(self.stable_id().cmp(&other.stable_id()))
    }
}

impl TakerBehaviour for AuctionOrder {
    fn with_updated_time(mut self, time: u64) -> Next<Self, Unit> {
        if let Some(span) = self.config.span_at_time(time) {
            self.current_span = span;
            self.fee = self.fee_for_input(self.input_amount);
            Next::Succ(self)
        } else {
            Next::Term(Unit)
        }
    }

    fn with_applied_trade(
        mut self,
        removed_input: InputAsset<u64>,
        added_output: OutputAsset<u64>,
    ) -> Next<Self, TerminalTake> {
        if added_output != self.exact_output_for_input(removed_input).unwrap_or(0) {
            return Next::Succ(self);
        }
        self.input_amount -= removed_input;
        self.output_amount += added_output;
        if self.input_amount == 0 {
            Next::Term(TerminalTake {
                remaining_input: self.input_amount,
                accumulated_output: self.output_amount,
                remaining_fee: self.fee,
                remaining_budget: self.execution_budget,
            })
        } else {
            Next::Succ(self)
        }
    }

    fn with_budget_corrected(mut self, delta: i64) -> (i64, Self) {
        let budget_remainder = self.execution_budget as i64;
        let corrected_remainder = budget_remainder + delta;
        let updated_budget_remainder = max(corrected_remainder, 0);
        let real_delta = updated_budget_remainder - budget_remainder;
        self.execution_budget = updated_budget_remainder as u64;
        (real_delta, self)
    }

    fn with_fee_charged(mut self, fee: u64) -> Self {
        let charged = fee.min(self.fee);
        self.fee -= charged;
        self.execution_budget = self.execution_budget.saturating_sub(charged);
        self
    }

    fn with_output_added(self, _added_output: u64) -> Self {
        // The on-chain auction validates an exact price equation, unlike limit orders.
        // Any "better than requested" excess would make the successor invalid.
        self
    }

    fn accepts_excess_output(&self) -> bool {
        false
    }

    fn exact_price_required(&self) -> bool {
        true
    }

    fn exact_output_for_input(&self, input: InputAsset<u64>) -> Option<OutputAsset<u64>> {
        let price = self.active_price();
        u64::try_from(input as u128 * *price.numer() / *price.denom()).ok()
    }

    fn try_terminate(self) -> Next<Self, TerminalTake> {
        if self.input_amount == 0 {
            Next::Term(TerminalTake {
                remaining_input: self.input_amount,
                accumulated_output: self.output_amount,
                remaining_fee: self.fee,
                remaining_budget: self.execution_budget,
            })
        } else {
            Next::Succ(self)
        }
    }
}

impl MarketTaker for AuctionOrder {
    type U = spectrum_cardano_lib::ex_units::ExUnits;

    fn side(&self) -> Side {
        side_of(self.config.base_asset, self.config.quote_asset)
    }

    fn input(&self) -> u64 {
        self.input_amount
    }

    fn output(&self) -> OutputAsset<u64> {
        self.output_amount
    }

    fn price(&self) -> AbsolutePrice {
        AbsolutePrice::from_price(self.side(), self.active_price())
    }

    fn operator_fee(&self, input_consumed: InputAsset<u64>) -> FeeAsset<u64> {
        self.fee_for_input(input_consumed)
    }

    fn fee(&self) -> FeeAsset<u64> {
        self.fee
    }

    fn budget(&self) -> FeeAsset<u64> {
        self.execution_budget
    }

    fn consumable_budget(&self) -> FeeAsset<u64> {
        0
    }

    fn marginal_cost_hint(&self) -> Self::U {
        spectrum_cardano_lib::ex_units::ExUnits::empty()
    }

    fn min_marginal_output(&self) -> OutputAsset<u64> {
        self.min_marginal_output
    }

    fn time_bounds(&self) -> TimeBounds<u64> {
        TimeBounds::After(self.config.start_time)
    }
}

impl Stable for AuctionOrder {
    type StableId = Token;

    fn stable_id(&self) -> Self::StableId {
        Token(self.id, AssetName::zero())
    }

    fn is_quasi_permanent(&self) -> bool {
        false
    }
}

impl SeqState for AuctionOrder {
    fn is_initial(&self) -> bool {
        true
    }
}

impl Tradable for AuctionOrder {
    type PairId = PairId;

    fn pair_id(&self) -> Self::PairId {
        PairId::canonical(self.config.base_asset, self.config.quote_asset)
    }
}

impl<C> TryFromLedger<TransactionOutput, C> for AuctionOrder
where
    C: Has<OutputRef> + Has<AuctionOrderRegistry> + Has<Time>,
{
    fn try_from_ledger(repr: &TransactionOutput, ctx: &C) -> Option<Self> {
        let script_hash = repr.script_hash()?;
        let entry = ctx.select::<AuctionOrderRegistry>().get(&script_hash)?.clone();
        let datum = repr.datum()?.into_pd()?;
        let config = AuctionOrderConfig::try_from_pd(datum.clone())?;
        if config.base_asset.is_native() || config.quote_asset.is_native() {
            return None;
        }
        let input_amount = repr.value().amount_of(config.base_asset)?;
        let output_amount = repr.value().amount_of(config.quote_asset).unwrap_or(0);
        let execution_budget = repr.value().amount_of(AssetClass::Native).unwrap_or(0);
        let current_time: u64 = ctx.select::<Time>().into();
        if current_time >= config.end_time() {
            return None;
        }
        let current_span = config.span_at_time(current_time).unwrap_or(0);
        let fee = fee_for_config_input(config, current_span, input_amount);
        Some(Self {
            id: order_id(ctx.select::<OutputRef>(), script_hash, datum),
            script_hash,
            config,
            input_amount,
            output_amount,
            execution_budget,
            fee,
            max_cost_per_ex_step: entry.max_cost_per_ex_step,
            min_marginal_output: entry.min_marginal_output,
            current_span,
        })
    }
}

fn fee_for_config_input(config: AuctionOrderConfig, span: u64, input_consumed: u64) -> u64 {
    let price = config.price_at_span(span);
    let quote = input_consumed as u128 * *price.numer() / *price.denom();
    (quote * *config.fee_per_quote.numer() / *config.fee_per_quote.denom()) as u64
}

impl TryFromPData for AuctionOrderConfig {
    fn try_from_pd(data: PlutusData) -> Option<Self> {
        let mut cpd = data.into_constr_pd()?;
        let start_time_ms = cpd.take_field(3)?.into_u64()?;
        let step_len_ms = cpd.take_field(4)?.into_u64()?;
        Some(Self {
            base_asset: AssetClass::try_from_pd(cpd.take_field(0)?)?,
            quote_asset: AssetClass::try_from_pd(cpd.take_field(1)?)?,
            price_start: RelativePrice::try_from_pd(cpd.take_field(2)?)?,
            start_time: start_time_ms / 1000,
            step_len: step_len_ms / 1000,
            steps: cpd.take_field(5)?.into_u64()?,
            price_decay_num: cpd.take_field(6)?.into_u128()?,
            fee_per_quote: RelativePrice::try_from_pd(cpd.take_field(7)?)?,
            redeemer: Ed25519KeyHash::from_raw_bytes(&*cpd.take_field(8)?.into_bytes()?).ok()?,
        })
    }
}

fn order_id(output_ref: OutputRef, script_hash: ScriptHash, datum: PlutusData) -> PolicyId {
    let mut bytes = Vec::new();
    bytes.extend(output_ref.tx_hash().to_raw_bytes());
    bytes.extend(output_ref.index().to_be_bytes());
    bytes.extend(script_hash.to_raw_bytes());
    bytes.extend(datum.to_cbor_bytes());
    blake2b224(&bytes).into()
}

pub fn exec_redeemer(span_ix: u64, successor_ix: u64) -> PlutusData {
    PlutusData::ConstrPlutusData(ConstrPlutusData {
        alternative: 0,
        fields: vec![span_ix.into_pd(), successor_ix.into_pd()],
        encodings: None,
    })
}

#[cfg(test)]
mod tests {
    use bloom_offchain::execution_engine::liquidity_book::types::RelativePrice;
    use num_rational::Ratio;

    use crate::orders::auction::AuctionOrderConfig;

    #[test]
    fn computes_decayed_price_for_span() {
        let conf = AuctionOrderConfig {
            price_start: RelativePrice::new_raw(5, 2),
            price_decay_num: 950,
            ..AuctionOrderConfig::default_for_tests()
        };

        assert_eq!(conf.price_at_span(0), Ratio::new_raw(5, 2));
        assert_eq!(conf.price_at_span(1), Ratio::new_raw(5 * 950, 2 * 1000));
        assert_eq!(
            conf.price_at_span(2),
            Ratio::new_raw(5 * 950 * 950, 2 * 1000 * 1000)
        );
    }
}
