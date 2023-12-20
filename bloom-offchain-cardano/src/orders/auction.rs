use std::cmp::Ordering;
use std::ops::Mul;

use cml_chain::builders::input_builder::SingleInputBuilder;
use cml_chain::builders::output_builder::SingleOutputBuilderResult;
use cml_chain::builders::redeemer_builder::RedeemerWitnessKey;
use cml_chain::builders::tx_builder::TransactionBuilder;
use cml_chain::builders::witness_builder::{PartialPlutusWitness, PlutusScriptWitness};
use cml_chain::certs::Credential;
use cml_chain::plutus::{ConstrPlutusData, ExUnits, PlutusData, RedeemerTag};
use cml_chain::transaction::TransactionOutput;
use cml_chain::utils::BigInt;
use cml_core::serialization::{LenEncoding, StringEncoding};
use cml_crypto::Ed25519KeyHash;
use num_rational::Ratio;
use void::Void;

use bloom_offchain::execution_engine::batch_exec::BatchExec;
use bloom_offchain::execution_engine::liquidity_book::fragment::Fragment;
use bloom_offchain::execution_engine::liquidity_book::side::SideM;
use bloom_offchain::execution_engine::liquidity_book::time::TimeBounds;
use bloom_offchain::execution_engine::liquidity_book::types::{BasePrice, ExecutionCost, Price};
use bloom_offchain::execution_engine::partial_fill::PartiallyFilled;
use spectrum_cardano_lib::output::FinalizedTxOut;
use spectrum_cardano_lib::transaction::TransactionOutputExtension;
use spectrum_cardano_lib::{AssetClass, NetworkTime};
use spectrum_offchain::data::Has;

use crate::orders::{Stateful, TLBCompatibleState};

const APPROX_AUCTION_COST: ExecutionCost = 1000;
const PRICE_DECAY_DEN: u64 = 10000;

pub const AUCTION_EXECUTION_UNITS: ExUnits = ExUnits {
    mem: 270000,
    steps: 140000000,
    encodings: None,
};

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct AuctionOrder {
    pub input_asset: AssetClass,
    pub input_amount: u64,
    pub output_asset: AssetClass,
    pub output_amount: u64,
    /// Price of input asset in output asset.
    pub start_price: Price,
    pub start_time: NetworkTime,
    pub step_len: u32,
    pub steps: u32,
    pub price_decay: u64,
    pub fee_per_quote: Ratio<u128>,
    pub redeemer: Ed25519KeyHash,
}

impl AuctionOrder {
    fn redeemer_cred(&self) -> Credential {
        Credential::PubKey {
            hash: self.redeemer,
            len_encoding: LenEncoding::Canonical,
            tag_encoding: None,
            hash_encoding: StringEncoding::Canonical,
        }
    }
    fn current_span_ix(&self, time: NetworkTime) -> Span {
        let step = self.step_len as u64;
        let span_ix = (time - self.start_time) / step;
        let low = self.start_time + step * span_ix;
        Span {
            index: span_ix as u16,
            lower_bound: low,
            upper_bound: low + step,
        }
    }
}

struct Span {
    index: u16,
    lower_bound: NetworkTime,
    upper_bound: NetworkTime,
}

impl PartialOrd for Stateful<AuctionOrder, TLBCompatibleState> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(&other))
    }
}

impl Ord for Stateful<AuctionOrder, TLBCompatibleState> {
    fn cmp(&self, other: &Self) -> Ordering {
        self.weight().cmp(&other.weight())
    }
}

impl Fragment for Stateful<AuctionOrder, TLBCompatibleState> {
    fn side(&self) -> SideM {
        self.state.side
    }

    fn input(&self) -> u64 {
        self.order.input_amount
    }

    fn price(&self) -> BasePrice {
        let current_span = (self.state.time_now - self.order.start_time) / (self.order.step_len as u64);
        let decay =
            Ratio::new(self.order.price_decay as u128, PRICE_DECAY_DEN as u128).pow(current_span as i32);
        let decay = match self.side() {
            SideM::Bid => decay.pow(-1),
            SideM::Ask => decay,
        };
        let base_price = BasePrice::from_price(self.side(), self.order.start_price);
        base_price * decay
    }

    fn weight(&self) -> u64 {
        let decay =
            Ratio::new(self.order.price_decay as u128, PRICE_DECAY_DEN as u128).pow(self.order.steps as i32);
        let terminal_price = self.order.start_price * decay;
        Ratio::new(self.order.input_amount as u128, 1)
            .mul(terminal_price)
            .mul(self.order.fee_per_quote)
            .to_integer() as u64
    }

    fn cost_hint(&self) -> ExecutionCost {
        APPROX_AUCTION_COST
    }

    fn time_bounds(&self) -> TimeBounds<u64> {
        let lower = self.order.start_time;
        let upper = self.order.start_time + (self.order.step_len * self.order.steps) as u64;
        TimeBounds::Within(lower, upper)
    }
}

// A newtype is needed to define instances in a proper place.
pub struct FullAuctionOrder<FilledOrd, Source>(pub FilledOrd, pub Source);

/// Execution logic for on-chain AO.
impl<Ctx, FO> BatchExec<TransactionBuilder, Option<TransactionOutput>, Ctx, Void>
    for FullAuctionOrder<FO, FinalizedTxOut>
where
    Ctx: Has<NetworkTime>,
    FO: PartiallyFilled<AuctionOrder>,
{
    fn try_exec(
        self,
        mut tx_builder: TransactionBuilder,
        context: Ctx,
    ) -> Result<(TransactionBuilder, Option<TransactionOutput>, Ctx), Void> {
        let FullAuctionOrder(filled_ord, FinalizedTxOut(consumed_out, in_ref)) = self;
        let mut candidate = consumed_out.clone();
        candidate.sub_asset(filled_ord.order().input_asset, filled_ord.removed_input());
        candidate.add_asset(filled_ord.order().output_asset, filled_ord.added_output());
        let residual_order = if filled_ord.has_terminated() {
            candidate.null_datum();
            let cred = filled_ord.order().redeemer_cred();
            candidate.update_payment_cred(cred);
            None
        } else {
            Some(candidate.clone())
        };
        // todo: replace `tx_builder.output_sizes()`
        let successor_ix = tx_builder.output_sizes().len();
        let span = filled_ord.order().current_span_ix(context.get::<NetworkTime>());
        let order_script = PartialPlutusWitness::new(
            PlutusScriptWitness::Ref(candidate.script_hash().unwrap()),
            auction_redeemer(span.index, successor_ix as u16),
        );
        let order_in = SingleInputBuilder::new(in_ref.into(), consumed_out)
            .plutus_script_inline_datum(order_script, Vec::new())
            .unwrap();
        tx_builder
            .add_output(SingleOutputBuilderResult::new(candidate))
            .unwrap();
        tx_builder.add_input(order_in).unwrap();
        // todo: make sure new bounds don't conflict with previous ones (if present).
        tx_builder.set_validity_start_interval(span.lower_bound);
        tx_builder.set_validity_start_interval(span.upper_bound);
        tx_builder.set_exunits(
            // todo: check for possible collisions bc of fixed 0-index.
            RedeemerWitnessKey::new(RedeemerTag::Spend, 0),
            AUCTION_EXECUTION_UNITS,
        );
        Ok((tx_builder, residual_order, context))
    }
}

fn auction_redeemer(span_ix: u16, successor_ix: u16) -> PlutusData {
    PlutusData::ConstrPlutusData(ConstrPlutusData::new(
        0,
        vec![
            PlutusData::Integer(BigInt::from(span_ix)),
            PlutusData::Integer(BigInt::from(successor_ix)),
        ],
    ))
}

#[cfg(test)]
mod tests {
    use cml_crypto::Ed25519KeyHash;
    use num_rational::Ratio;

    use bloom_offchain::execution_engine::liquidity_book::fragment::Fragment;
    use bloom_offchain::execution_engine::liquidity_book::side::SideM;
    use bloom_offchain::execution_engine::liquidity_book::types::BasePrice;
    use spectrum_cardano_lib::AssetClass;

    use crate::orders::auction::{AuctionOrder, PRICE_DECAY_DEN};
    use crate::orders::{Stateful, TLBCompatibleState};

    #[test]
    fn correct_price_decay_as_time_advances_ask() {
        correct_price_decay_as_time_advances(SideM::Ask)
    }

    #[test]
    fn correct_price_decay_as_time_advances_bid() {
        correct_price_decay_as_time_advances(SideM::Bid)
    }

    fn correct_price_decay_as_time_advances(side: SideM) {
        let o = AuctionOrder {
            input_asset: AssetClass::Native,
            input_amount: 10_000_000_000,
            output_asset: AssetClass::Native,
            output_amount: 0,
            start_price: Ratio::new(100, 1),
            start_time: 0,
            step_len: 1,
            steps: 10,
            price_decay: 9900, // reduction of 1pp with each step
            fee_per_quote: Ratio::new(10, 1),
            redeemer: Ed25519KeyHash::from([0u8; 28]),
        };
        let init_state = TLBCompatibleState { side, time_now: 0 };
        let term_state = TLBCompatibleState { side, time_now: 10 };
        let term_price =
            o.start_price * Ratio::new(o.price_decay as u128, PRICE_DECAY_DEN as u128).pow(o.steps as i32);
        assert_eq!(
            Stateful::new(o, init_state).price(),
            BasePrice::from_price(side, o.start_price)
        );
        assert_eq!(
            Stateful::new(o, term_state).price(),
            BasePrice::from_price(side, term_price)
        );
    }
}
