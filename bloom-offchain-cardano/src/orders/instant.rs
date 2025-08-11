use crate::orders::harden_price;
use crate::orders::limit::{order_state, LimitOrderValidation, OrderState, MIN_LOVELACE};
use bloom_offchain::execution_engine::liquidity_book::core::{Next, TerminalTake, Unit};
use bloom_offchain::execution_engine::liquidity_book::linear_output_relative;
use bloom_offchain::execution_engine::liquidity_book::market_taker::{MarketTaker, TakerBehaviour};
use bloom_offchain::execution_engine::liquidity_book::side::Side;
use bloom_offchain::execution_engine::liquidity_book::time::TimeBounds;
use bloom_offchain::execution_engine::liquidity_book::types::{
    AbsolutePrice, FeeAsset, InputAsset, Lovelace, OutputAsset, RelativePrice,
};
use bloom_offchain::execution_engine::liquidity_book::weight::Weighted;
use cml_chain::assets::PositiveCoin;
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
use std::cmp::{max, Ordering};
use std::fmt::{Display, Formatter};

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
    pub beacon: Token,
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

impl InstantOrder {
    pub fn beacon_from_oref(oref: OutputRef) -> Token {
        let mut pseudo_beacon: [u8; 60] = [0u8; 60];
        let mut tx_hash_with_index = oref.tx_hash().to_raw_bytes().to_vec();
        tx_hash_with_index.extend_from_slice(oref.index().to_be_bytes().as_ref());
        pseudo_beacon[..tx_hash_with_index.len()].copy_from_slice(&tx_hash_with_index);
        Token::from(pseudo_beacon)
    }
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
        Next::Term(TerminalTake {
            remaining_input: self.input_amount,
            accumulated_output: self.output_amount,
            remaining_fee: self.fee,
            remaining_budget: self.execution_budget,
        })
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
        Next::Term(TerminalTake {
            remaining_input: self.input_amount,
            accumulated_output: self.output_amount,
            remaining_fee: self.fee,
            remaining_budget: self.execution_budget,
        })
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
        self.execution_budget
    }

    fn marginal_cost_hint(&self) -> ExUnits {
        self.marginal_cost
    }

    fn min_marginal_output(&self) -> OutputAsset<u64> {
        self.base_price.to_integer() as u64
    }

    fn time_bounds(&self) -> TimeBounds<u64> {
        TimeBounds::None
    }
}

impl Stable for InstantOrder {
    type StableId = Token;
    fn stable_id(&self) -> Self::StableId {
        self.beacon
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

#[derive(Copy, Clone, Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstantOrderValidation {
    pub min_lovelace: u64,
    pub min_fee_lovelace: Lovelace,
    pub min_execution_budget_lovelace: Lovelace,
}

#[derive(Debug, PartialEq, Eq)]
struct Datum {
    pub redeemer_address: PlutusAddress,
    pub input: AssetClass,
    pub output: AssetClass,
    pub base_price: RelativePrice,
    pub fee: FeeAsset<u64>,
    pub min_lovelace: Lovelace,
    pub permitted_executor: Ed25519KeyHash,
    pub cancellation_pkh: Ed25519KeyHash,
    pub cancellation_after: u64,
}

struct DatumMapping {
    pub redeemer_address: usize,
    pub input: usize,
    pub output: usize,
    pub base_price: usize,
    pub fee: usize,
    pub min_lovelace: usize,
    pub permitted_executor: usize,
    pub cancellation_after: usize,
    pub cancellation_pkh: usize,
}

const DATUM_MAPPING: DatumMapping = DatumMapping {
    redeemer_address: 1,
    input: 2,
    output: 3,
    base_price: 4,
    fee: 5,
    min_lovelace: 6,
    permitted_executor: 7,
    cancellation_after: 8,
    cancellation_pkh: 9,
};

impl TryFromPData for Datum {
    fn try_from_pd(data: PlutusData) -> Option<Self> {
        let mut cpd = data.into_constr_pd()?;
        let input = AssetClass::try_from_pd(cpd.take_field(DATUM_MAPPING.input)?)?;
        let output = AssetClass::try_from_pd(cpd.take_field(DATUM_MAPPING.output)?)?;
        let base_price = RelativePrice::try_from_pd(cpd.take_field(DATUM_MAPPING.base_price)?)?;
        let fee = cpd.take_field(DATUM_MAPPING.fee)?.into_u64()?;
        let min_lovelace = cpd.take_field(DATUM_MAPPING.min_lovelace)?.into_u64()?;
        let redeemer_address = PlutusAddress::try_from_pd(cpd.take_field(DATUM_MAPPING.redeemer_address)?)?;
        let cancellation_pkh =
            Ed25519KeyHash::from_raw_bytes(&*cpd.take_field(DATUM_MAPPING.cancellation_pkh)?.into_bytes()?)
                .ok()?;
        let permitted_executor =
            Ed25519KeyHash::from_raw_bytes(&*cpd.take_field(DATUM_MAPPING.permitted_executor)?.into_bytes()?)
                .ok()?;
        let cancellation_after = cpd.take_field(DATUM_MAPPING.cancellation_after)?.into_u64()?;
        Some(Datum {
            redeemer_address,
            input,
            output,
            base_price,
            fee,
            min_lovelace,
            cancellation_pkh,
            permitted_executor,
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
        + Has<InstantOrderValidation>,
{
    fn try_from_ledger(repr: &TransactionOutput, ctx: &C) -> Option<Self> {
        if test_address(repr.address(), ctx) {
            let value = repr.value().clone();
            let datum = repr.datum()?.into_pd()?;
            let conf = Datum::try_from_pd(datum.clone())?;
            let total_input_asset_amount = value.amount_of(conf.input)?;

            let tradable_input: u64 = match conf.input {
                AssetClass::Native => value.coin - conf.fee - conf.min_lovelace,
                token => value.amount_of(token)?,
            };

            if let Some(_) = linear_output_relative(tradable_input, conf.base_price) {
                let sufficient_input = total_input_asset_amount >= tradable_input;
                let executable = conf.permitted_executor == ctx.select::<OperatorCred>().into();
                let validation = ctx.select::<InstantOrderValidation>();
                let valid_configuration = conf.min_lovelace >= validation.min_lovelace;
                let sufficient_fee =
                    conf.fee >= (validation.min_execution_budget_lovelace + validation.min_fee_lovelace);
                if sufficient_input && executable && valid_configuration {
                    let output_ref = ctx.select::<OutputRef>();
                    let script_info = ctx.select::<DeployedScriptInfo<{ InstantOrderV1 as u8 }>>();
                    return Some(InstantOrder {
                        beacon: InstantOrder::beacon_from_oref(output_ref),
                        input_asset: conf.input,
                        input_amount: tradable_input,
                        output_asset: conf.output,
                        output_amount: value.amount_of(conf.output).unwrap_or(0),
                        base_price: harden_price(conf.base_price, tradable_input),
                        execution_budget: validation.min_execution_budget_lovelace,
                        fee_asset: AssetClass::Native,
                        redeemer_address: conf.redeemer_address,
                        cancellation_pkh: conf.cancellation_pkh,
                        marginal_cost: script_info.marginal_cost,
                        virgin: true,
                        cancellation_after: conf.cancellation_after,
                        fee: conf.fee - validation.min_execution_budget_lovelace,
                    });
                } else {
                    trace!(
                        "UTxO {}, InstantOrder :: sufficient_input: {}, sufficient_fee: {}, executable: {}, valid_configuration: {}",
                        ctx.select::<OutputRef>(),
                        sufficient_input,
                        sufficient_fee,
                        executable,
                        valid_configuration,
                    );
                }
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use crate::orders::instant::{InstantOrder, InstantOrderValidation};
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

    impl Has<InstantOrderValidation> for Context {
        fn select<U: IsEqual<InstantOrderValidation>>(&self) -> InstantOrderValidation {
            InstantOrderValidation {
                min_lovelace: 0,
                min_fee_lovelace: 0,
                min_execution_budget_lovelace: 0,
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
            cred: OperatorCred(
                Ed25519KeyHash::from_hex("15772e8f1fdcf12d59636caf42522b7d6249ccb223253eb7e9b6d509").unwrap(),
            ),
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

    const ORDER_UTXO: &str = "a300581d70d9143ac63473b17a215d1b7484dfb6ac6b4a0005beb0e26a6ca02c9601821a00632ea0a1581c77cb34f72da105bd0cab41c2a10e2fa2fe97a181e6771a62d0c9673ea14974657374546f6b656e1a00989680028201d81858f7d8799f4101d8799fd8799f581caf31ce038e8a8b297546db1a86b3973c32e49e191b3c76626046ed93ffd8799fd8799fd8799f581c1fc3c30bb2966c801399aa685012979d1f5048da659407e8371b8126ffffffffd8799f581c77cb34f72da105bd0cab41c2a10e2fa2fe97a181e6771a62d0c9673e4974657374546f6b656effd8799f581cce93f37e1b9da84739be6b32d266f5c7eef5b56ee20173e64fc6ec8945746f6b656effd8799f0001ff1a0007a1201a0016e360581c15772e8f1fdcf12d59636caf42522b7d6249ccb223253eb7e9b6d50900581caf31ce038e8a8b297546db1a86b3973c32e49e191b3c76626046ed93ff";
}
