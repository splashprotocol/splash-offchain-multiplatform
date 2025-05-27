use std::cmp::{max, Ordering};
use std::fmt::{Display, Formatter};

use crate::orders::harden_price;
use crate::orders::limit::{order_state, LimitOrderValidation, OrderState, MIN_LOVELACE};
use bloom_offchain::execution_engine::liquidity_book::core::{Next, TerminalTake, Unit};
use bloom_offchain::execution_engine::liquidity_book::linear_output_relative;
use bloom_offchain::execution_engine::liquidity_book::market_taker::{MarketTaker, TakerBehaviour};
use bloom_offchain::execution_engine::liquidity_book::side::Side;
use bloom_offchain::execution_engine::liquidity_book::time::TimeBounds;
use bloom_offchain::execution_engine::liquidity_book::types::{
    AbsolutePrice, FeeAsset, InputAsset, OutputAsset, RelativePrice,
};
use bloom_offchain::execution_engine::liquidity_book::weight::Weighted;
use cml_chain::plutus::{ConstrPlutusData, PlutusData};
use cml_chain::transaction::TransactionOutput;
use cml_chain::PolicyId;
use cml_core::serialization::Serialize;
use cml_crypto::{blake2b224, Ed25519KeyHash, RawBytesEncoding};
use log::trace;
use spectrum_cardano_lib::address::PlutusAddress;
use spectrum_cardano_lib::ex_units::ExUnits;
use spectrum_cardano_lib::plutus_data::{
    ConstrPlutusDataExtension, DatumExtension, IntoPlutusData, PlutusDataExtension,
};
use spectrum_cardano_lib::transaction::TransactionOutputExtension;
use spectrum_cardano_lib::types::TryFromPData;
use spectrum_cardano_lib::value::ValueExtension;
use spectrum_cardano_lib::{AssetClass, AssetName, OutputRef, Token};
use spectrum_offchain::domain::{Has, SeqState, Stable, Tradable};
use spectrum_offchain::ledger::TryFromLedger;
use spectrum_offchain_cardano::creds::OperatorCred;
use spectrum_offchain_cardano::data::pair::{side_of, PairId};
use spectrum_offchain_cardano::deployment::ProtocolValidator::InstantOrderV1;
use spectrum_offchain_cardano::deployment::{test_address, DeployedScriptInfo};
use spectrum_offchain_cardano::handler_context::{ConsumedIdentifiers, ConsumedInputs};

pub const EXEC_REDEEMER: PlutusData = PlutusData::ConstrPlutusData(ConstrPlutusData {
    alternative: 1,
    fields: vec![],
    encodings: None,
});

/// A version of limit order optimized for immediate execution.
/// Can be executed at a configured or better price as long as there is enough budget.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct InstantOrder {
    /// Identifier of the order.
    pub beacon: PolicyId,
    /// What a user pays.
    pub input_asset: AssetClass,
    /// Remaining tradable input.
    pub input_amount: InputAsset<u64>,
    /// What a user receives.
    pub output_asset: AssetClass,
    /// Accumulated output.
    pub output_amount: OutputAsset<u64>,
    /// Worst acceptable price (Output/Input).
    pub base_price: RelativePrice,
    /// Currency used to pay for execution.
    pub fee_asset: AssetClass,
    /// Remaining ADA to facilitate execution.
    pub execution_budget: FeeAsset<u64>,
    /// Fee reserved for the whole swap.
    pub fee: FeeAsset<u64>,
    /// Assumed cost (in Lovelace) of one step of execution.
    pub max_cost_per_ex_step: FeeAsset<u64>,
    /// Minimal marginal output allowed per execution step.
    pub min_marginal_output: OutputAsset<u64>,
    /// Redeemer address.
    pub redeemer_address: PlutusAddress,
    /// Cancellation PKH.
    pub cancellation_pkh: Ed25519KeyHash,
    /// How many execution units each order consumes.
    pub marginal_cost: ExUnits,
    /// If this state is untouched.
    pub virgin: bool,
    /// Order cannot be canceled before this point.
    pub cancellation_after: u64,
}

impl Display for InstantOrder {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(
            format!(
                "InstantOrder({}, {}, {}, p={}, in={} {}, out={} {}, budget={}, fee={} {}, init={})",
                self.beacon,
                self.side(),
                self.pair_id(),
                self.price(),
                self.input_amount,
                self.input_asset,
                self.output_amount,
                self.output_asset,
                self.execution_budget,
                self.fee,
                self.fee_asset,
                self.virgin,
            )
            .as_str(),
        )
    }
}

impl PartialOrd for InstantOrder {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for InstantOrder {
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

impl TakerBehaviour for InstantOrder {
    fn with_updated_time(self, _: u64) -> Next<Self, Unit> {
        Next::Succ(self)
    }

    fn with_applied_trade(
        mut self,
        removed_input: InputAsset<u64>,
        added_output: OutputAsset<u64>,
    ) -> Next<Self, TerminalTake> {
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
        self.fee -= fee;
        self
    }

    fn with_output_added(mut self, added_output: u64) -> Self {
        self.output_amount += added_output;
        self
    }

    fn try_terminate(self) -> Next<Self, TerminalTake> {
        if self.execution_budget < self.max_cost_per_ex_step {
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

impl MarketTaker for InstantOrder {
    type U = ExUnits;

    fn side(&self) -> Side {
        side_of(self.input_asset, self.output_asset)
    }

    fn input(&self) -> u64 {
        self.input_amount
    }

    fn output(&self) -> OutputAsset<u64> {
        self.output_amount
    }

    fn price(&self) -> AbsolutePrice {
        AbsolutePrice::from_price(self.side(), self.base_price)
    }

    fn operator_fee(&self, input_consumed: InputAsset<u64>) -> FeeAsset<u64> {
        self.fee
            .saturating_mul(input_consumed)
            .checked_div(self.input_amount)
            .unwrap_or(0)
    }

    fn fee(&self) -> FeeAsset<u64> {
        self.fee
    }

    fn budget(&self) -> FeeAsset<u64> {
        self.execution_budget
    }

    fn consumable_budget(&self) -> FeeAsset<u64> {
        self.max_cost_per_ex_step
    }

    fn marginal_cost_hint(&self) -> ExUnits {
        self.marginal_cost
    }

    fn min_marginal_output(&self) -> OutputAsset<u64> {
        self.min_marginal_output
    }

    fn time_bounds(&self) -> TimeBounds<u64> {
        TimeBounds::None
    }
}

impl Stable for InstantOrder {
    type StableId = Token;
    fn stable_id(&self) -> Self::StableId {
        Token(self.beacon, AssetName::zero())
    }
    fn is_quasi_permanent(&self) -> bool {
        false
    }
}

impl SeqState for InstantOrder {
    fn is_initial(&self) -> bool {
        self.virgin
    }
}

impl Tradable for InstantOrder {
    type PairId = PairId;

    fn pair_id(&self) -> Self::PairId {
        PairId::canonical(self.input_asset, self.output_asset)
    }
}

#[derive(Debug, PartialEq, Eq)]
struct Datum {
    pub beacon: PolicyId,
    pub input: AssetClass,
    pub tradable_input: InputAsset<u64>,
    pub cost_per_ex_step: FeeAsset<u64>,
    pub output: AssetClass,
    pub base_price: RelativePrice,
    pub fee: FeeAsset<u64>,
    pub redeemer_address: PlutusAddress,
    pub cancellation_pkh: Ed25519KeyHash,
    pub authed_executor: Ed25519KeyHash,
    pub cancellation_after: u64,
}

struct DatumMapping {
    pub beacon: usize,
    pub input: usize,
    pub tradable_input: usize,
    pub cost_per_ex_step: usize,
    pub output: usize,
    pub base_price: usize,
    pub fee: usize,
    pub redeemer_address: usize,
    pub cancellation_pkh: usize,
    pub authed_executor: usize,
    pub cancellation_after: usize,
}

const DATUM_MAPPING: DatumMapping = DatumMapping {
    redeemer_address: 1,
    input: 2,
    tradable_input: 3,
    cost_per_ex_step: 4,
    output: 5,
    base_price: 6,
    fee: 7,
    authed_executor: 8,
    cancellation_after: 9,
    cancellation_pkh: 10,
    beacon: 11,
};

pub fn unsafe_update_datum(data: &mut PlutusData, tradable_input: InputAsset<u64>, fee: FeeAsset<u64>) {
    let cpd = data.get_constr_pd_mut().unwrap();
    cpd.set_field(DATUM_MAPPING.tradable_input, tradable_input.into_pd());
    cpd.set_field(DATUM_MAPPING.fee, fee.into_pd());
}

impl TryFromPData for Datum {
    fn try_from_pd(data: PlutusData) -> Option<Self> {
        let mut cpd = data.into_constr_pd()?;
        let beacon = PolicyId::from_raw_bytes(&*cpd.take_field(DATUM_MAPPING.beacon)?.into_bytes()?).ok()?;
        let input = AssetClass::try_from_pd(cpd.take_field(DATUM_MAPPING.input)?)?;
        let tradable_input = cpd.take_field(DATUM_MAPPING.tradable_input)?.into_u64()?;
        let cost_per_ex_step = cpd.take_field(DATUM_MAPPING.cost_per_ex_step)?.into_u64()?;
        let output = AssetClass::try_from_pd(cpd.take_field(DATUM_MAPPING.output)?)?;
        let base_price = RelativePrice::try_from_pd(cpd.take_field(DATUM_MAPPING.base_price)?)?;
        let fee = cpd.take_field(DATUM_MAPPING.fee)?.into_u64()?;
        let redeemer_address = PlutusAddress::try_from_pd(cpd.take_field(DATUM_MAPPING.redeemer_address)?)?;
        let cancellation_pkh =
            Ed25519KeyHash::from_raw_bytes(&*cpd.take_field(DATUM_MAPPING.cancellation_pkh)?.into_bytes()?)
                .ok()?;
        let authed_executor =
            Ed25519KeyHash::from_raw_bytes(&*cpd.take_field(DATUM_MAPPING.authed_executor)?.into_bytes()?)
                .ok()?;
        let cancellation_after = cpd.take_field(DATUM_MAPPING.cancellation_after)?.into_u64()?;
        Some(Datum {
            beacon,
            input,
            tradable_input,
            cost_per_ex_step,
            output,
            base_price,
            fee,
            redeemer_address,
            cancellation_pkh,
            authed_executor,
            cancellation_after,
        })
    }
}

const MIN_EXECUTION_STEPS: u64 = 1;
const MIN_MARGINAL_OUTPUT_FACTOR: u64 = 2;

impl<C> TryFromLedger<TransactionOutput, C> for InstantOrder
where
    C: Has<OperatorCred>
        + Has<OutputRef>
        + Has<ConsumedIdentifiers<Token>>
        + Has<ConsumedInputs>
        + Has<DeployedScriptInfo<{ InstantOrderV1 as u8 }>>
        + Has<LimitOrderValidation>,
{
    fn try_from_ledger(repr: &TransactionOutput, ctx: &C) -> Option<Self> {
        if test_address(repr.address(), ctx) {
            let value = repr.value().clone();
            let datum = repr.datum()?.into_pd()?;
            let conf = Datum::try_from_pd(datum.clone())?;
            let total_input_asset_amount = value.amount_of(conf.input)?;
            let total_ada_input = value.amount_of(AssetClass::Native)?;
            let (reserved_lovelace, tradable_lovelace) = match (conf.input, conf.output) {
                (AssetClass::Native, _) => (MIN_LOVELACE, conf.tradable_input),
                (_, AssetClass::Native) => (0, 0),
                _ => (MIN_LOVELACE, 0),
            };
            let execution_budget = total_ada_input
                .checked_sub(reserved_lovelace)
                .and_then(|lov| lov.checked_sub(conf.fee))
                .and_then(|lov| lov.checked_sub(tradable_lovelace))?;
            if let Some(base_output) = linear_output_relative(conf.tradable_input, conf.base_price) {
                let min_marginal_output = base_output / MIN_MARGINAL_OUTPUT_FACTOR;
                let max_execution_steps_available = execution_budget.checked_div(conf.cost_per_ex_step)?;
                let sufficient_input = total_input_asset_amount >= conf.tradable_input;
                let sufficient_execution_budget = max_execution_steps_available >= MIN_EXECUTION_STEPS;
                let executable = conf.authed_executor == ctx.select::<OperatorCred>().into();
                let validation = ctx.select::<LimitOrderValidation>();
                let valid_configuration = conf.cost_per_ex_step >= validation.min_cost_per_ex_step
                    && execution_budget >= conf.cost_per_ex_step;
                let order_state = order_state(conf.beacon, datum, DATUM_MAPPING.beacon, ctx);
                let sufficient_fee = match order_state {
                    Some(OrderState::New) | None => conf.fee >= validation.min_fee_lovelace,
                    _ => true,
                };
                let valid_beacon = order_state.is_some();
                if sufficient_input
                    && sufficient_execution_budget
                    && sufficient_fee
                    && executable
                    && valid_configuration
                    && valid_beacon
                {
                    // Fresh beacon must be derived from one of consumed utxos.
                    let script_info = ctx.select::<DeployedScriptInfo<{ InstantOrderV1 as u8 }>>();
                    return Some(InstantOrder {
                        beacon: conf.beacon,
                        input_asset: conf.input,
                        input_amount: conf.tradable_input,
                        output_asset: conf.output,
                        output_amount: value.amount_of(conf.output).unwrap_or(0),
                        base_price: harden_price(conf.base_price, conf.tradable_input),
                        execution_budget,
                        fee_asset: AssetClass::Native,
                        fee: conf.fee,
                        max_cost_per_ex_step: conf.cost_per_ex_step,
                        min_marginal_output,
                        redeemer_address: conf.redeemer_address,
                        cancellation_pkh: conf.cancellation_pkh,
                        marginal_cost: script_info.marginal_cost,
                        virgin: matches!(order_state, Some(OrderState::New)),
                        cancellation_after: conf.cancellation_after,
                    });
                } else {
                    trace!(
                            "UTxO {}, InstantOrder {} :: sufficient_input: {}, sufficient_execution_budget: {}, sufficient_fee: {}, executable: {}, valid_configuration: {}, is_valid_beacon: {}",
                            ctx.select::<OutputRef>(),
                            conf.beacon,
                            sufficient_input,
                            sufficient_execution_budget,
                            sufficient_fee,
                            executable,
                            valid_configuration,
                            valid_beacon
                        );
                }
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use crate::orders::instant::InstantOrder;
    use crate::orders::limit::LimitOrderValidation;
    use bloom_offchain::execution_engine::liquidity_book::market_taker::MarketTaker;
    use cml_chain::transaction::TransactionOutput;
    use cml_core::serialization::Deserialize;
    use cml_crypto::{Ed25519KeyHash, TransactionHash};
    use spectrum_cardano_lib::{OutputRef, Token};
    use spectrum_offchain::data::small_vec::SmallVec;
    use spectrum_offchain::display::display_option;
    use spectrum_offchain::domain::Has;
    use spectrum_offchain::ledger::TryFromLedger;
    use spectrum_offchain_cardano::creds::OperatorCred;
    use spectrum_offchain_cardano::deployment::ProtocolValidator::InstantOrderV1;
    use spectrum_offchain_cardano::deployment::{
        DeployedScriptInfo, DeployedValidators, ProtocolScriptHashes,
    };
    use spectrum_offchain_cardano::handler_context::{
        ConsumedIdentifiers, ConsumedInputs, ProducedIdentifiers,
    };
    use type_equalities::IsEqual;

    struct Context {
        oref: OutputRef,
        instant_order: DeployedScriptInfo<{ InstantOrderV1 as u8 }>,
        cred: OperatorCred,
        consumed_inputs: ConsumedInputs,
        consumed_identifiers: ConsumedIdentifiers<Token>,
        produced_identifiers: ProducedIdentifiers<Token>,
    }

    impl Has<OutputRef> for Context {
        fn select<U: IsEqual<OutputRef>>(&self) -> OutputRef {
            self.oref
        }
    }

    impl Has<ConsumedIdentifiers<Token>> for Context {
        fn select<U: IsEqual<ConsumedIdentifiers<Token>>>(&self) -> ConsumedIdentifiers<Token> {
            self.consumed_identifiers
        }
    }

    impl Has<ProducedIdentifiers<Token>> for Context {
        fn select<U: IsEqual<ProducedIdentifiers<Token>>>(&self) -> ProducedIdentifiers<Token> {
            self.produced_identifiers
        }
    }

    impl Has<LimitOrderValidation> for Context {
        fn select<U: IsEqual<LimitOrderValidation>>(&self) -> LimitOrderValidation {
            LimitOrderValidation {
                min_cost_per_ex_step: 0,
                min_fee_lovelace: 0,
            }
        }
    }

    impl Has<ConsumedInputs> for Context {
        fn select<U: IsEqual<ConsumedInputs>>(&self) -> ConsumedInputs {
            self.consumed_inputs
        }
    }

    impl Has<OperatorCred> for Context {
        fn select<U: IsEqual<OperatorCred>>(&self) -> OperatorCred {
            self.cred
        }
    }

    impl Has<DeployedScriptInfo<{ InstantOrderV1 as u8 }>> for Context {
        fn select<U: IsEqual<DeployedScriptInfo<{ InstantOrderV1 as u8 }>>>(
            &self,
        ) -> DeployedScriptInfo<{ InstantOrderV1 as u8 }> {
            self.instant_order
        }
    }

    #[test]
    fn try_read() {
        const TX: &str = "b72f29953347030c5051bdba801d2c5dfdebc9574dfb28e139d0ef131033aee6";
        const IX: u64 = 0;
        let oref = OutputRef::new(TransactionHash::from_hex(TX).unwrap(), IX);
        const TX_BC: &str = "99d8460a4f4c500bccd922e34db1536d784b0b5952fa8bc1bc90d48333344dde";
        const IX_BC: u64 = 1;
        let oref_bc = OutputRef::new(TransactionHash::from_hex(TX_BC).unwrap(), IX_BC);
        let raw_deployment = std::fs::read_to_string("/Users/oskin/dev/spectrum/spectrum-offchain-multiplatform/bloom-cardano-agent/resources/mainnet.deployment.json").expect("Cannot load deployment file");
        let deployment: DeployedValidators =
            serde_json::from_str(&raw_deployment).expect("Invalid deployment file");
        let scripts = ProtocolScriptHashes::from(&deployment);
        let ctx = Context {
            oref,
            instant_order: scripts.instant_order,
            cred: OperatorCred(Ed25519KeyHash::from([0u8; 28])),
            consumed_inputs: SmallVec::new(vec![oref_bc].into_iter()).into(),
            consumed_identifiers: SmallVec::new(
                vec![Token::from_string_unsafe(
                    "64b18826b8f4e3c6a870c84dcf10370b91f4add2550c92e061db356b.",
                )]
                .into_iter(),
            )
            .into(),
            produced_identifiers: Default::default(),
        };
        let bearer = TransactionOutput::from_cbor_bytes(&*hex::decode(ORDER_UTXO).unwrap()).unwrap();
        let ord = InstantOrder::try_from_ledger(&bearer, &ctx);
        println!("Order: {}", display_option(&ord));
        println!("P_abs: {}", display_option(&ord.map(|x| x.price())));
    }

    const ORDER_UTXO: &str = "a30058391164956ddc4df888a294bec79d53a91601b60fc46592e8b78e33a486ff7846f6bb07f5b2825885e4502679e699b4e60a0c4609a46bc35454cd011a0036ee80028201d81858f6d8798c4101d87982d87981581c719bee424a97b58b3dca88fe5da6feac6494aa7226f975f3506c5b25d87981d87981d87981581c7846f6bb07f5b2825885e4502679e699b4e60a0c4609a46bc35454cdd8798240401a000f42401a000927c0d87982581c41f4454459daa1b6b856a7a5e28e6ea930bf9d593adec38d7700f7df4442415348d879821a001dad0d1a009896801a0007a120581cedbf33f5d6e083970648e39175c49ec1c093df76b6e6a0f1473e47761a68345fc9581c719bee424a97b58b3dca88fe5da6feac6494aa7226f975f3506c5b25581ca83d20206ee7e3ae5cabfbdb6e026f53f5220dcc8980f1b10784530c";
}
