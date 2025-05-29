use std::cmp::{max, min, Ordering};
use std::fmt::{Display, Formatter};

use crate::orders::harden_price;
use bloom_offchain::execution_engine::liquidity_book::core::{Next, TerminalTake, Unit};
use bloom_offchain::execution_engine::liquidity_book::linear_output_relative;
use bloom_offchain::execution_engine::liquidity_book::market_taker::{MarketTaker, TakerBehaviour};
use bloom_offchain::execution_engine::liquidity_book::side::Side;
use bloom_offchain::execution_engine::liquidity_book::time::TimeBounds;
use bloom_offchain::execution_engine::liquidity_book::types::{
    AbsolutePrice, FeeAsset, InputAsset, Lovelace, OutputAsset, RelativePrice,
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
use spectrum_offchain_cardano::deployment::ProtocolValidator::LimitOrderV1;
use spectrum_offchain_cardano::deployment::{test_address, DeployedScriptInfo};
use spectrum_offchain_cardano::handler_context::{ConsumedIdentifiers, ConsumedInputs, ProducedIdentifiers};

pub const EXEC_REDEEMER: PlutusData = PlutusData::ConstrPlutusData(ConstrPlutusData {
    alternative: 1,
    fields: vec![],
    encodings: None,
});

/// Composable limit order. Can be executed at a configured
/// or better price as long as there is enough budget.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct LimitOrder {
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
    /// Redeemer address.
    pub redeemer_address: PlutusAddress,
    /// Cancellation PKH.
    pub cancellation_pkh: Ed25519KeyHash,
    /// Is executor's signature required.
    pub requires_executor_sig: bool,
    /// How many execution units each order consumes.
    pub marginal_cost: ExUnits,
    /// If this state is untouched.
    pub virgin: bool,
}

impl Display for LimitOrder {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(
            format!(
                "LimitOrder({}, {}, {}, p={}, in={} {}, out={} {}, budget={}, fee={} {}, init={})",
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

impl PartialOrd for LimitOrder {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for LimitOrder {
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

impl TakerBehaviour for LimitOrder {
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

impl MarketTaker for LimitOrder {
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

impl Stable for LimitOrder {
    type StableId = Token;
    fn stable_id(&self) -> Self::StableId {
        Token(self.beacon, AssetName::zero())
    }
    fn is_quasi_permanent(&self) -> bool {
        false
    }
}

impl SeqState for LimitOrder {
    fn is_initial(&self) -> bool {
        self.virgin
    }
}

impl Tradable for LimitOrder {
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
    pub min_marginal_output: OutputAsset<u64>,
    pub output: AssetClass,
    pub base_price: RelativePrice,
    pub fee: FeeAsset<u64>,
    pub redeemer_address: PlutusAddress,
    pub cancellation_pkh: Ed25519KeyHash,
    pub permitted_executors: Vec<Ed25519KeyHash>,
}

struct DatumMapping {
    pub beacon: usize,
    pub input: usize,
    pub tradable_input: usize,
    pub cost_per_ex_step: usize,
    pub min_marginal_output: usize,
    pub output: usize,
    pub base_price: usize,
    pub fee: usize,
    pub redeemer_address: usize,
    pub cancellation_pkh: usize,
    pub permitted_executors: usize,
}

const DATUM_MAPPING: DatumMapping = DatumMapping {
    beacon: 1,
    input: 2,
    tradable_input: 3,
    cost_per_ex_step: 4,
    min_marginal_output: 5,
    output: 6,
    base_price: 7,
    fee: 8,
    redeemer_address: 9,
    cancellation_pkh: 10,
    permitted_executors: 11,
};

pub fn unsafe_update_datum(data: &mut PlutusData, tradable_input: InputAsset<u64>, fee: FeeAsset<u64>) {
    let cpd = data.get_constr_pd_mut().unwrap();
    cpd.set_field(DATUM_MAPPING.tradable_input, tradable_input.into_pd());
    cpd.set_field(DATUM_MAPPING.fee, fee.into_pd());
}

pub(super) fn with_erased_beacon_unsafe(data: PlutusData, index: usize) -> PlutusData {
    let mut cpd = data.into_constr_pd().unwrap();
    cpd.set_field(index, [0u8; 28].into_pd());
    cpd.into_pd()
}

impl TryFromPData for Datum {
    fn try_from_pd(data: PlutusData) -> Option<Self> {
        let mut cpd = data.into_constr_pd()?;
        let beacon = PolicyId::from_raw_bytes(&*cpd.take_field(DATUM_MAPPING.beacon)?.into_bytes()?).ok()?;
        let input = AssetClass::try_from_pd(cpd.take_field(DATUM_MAPPING.input)?)?;
        let tradable_input = cpd.take_field(DATUM_MAPPING.tradable_input)?.into_u64()?;
        let cost_per_ex_step = cpd.take_field(DATUM_MAPPING.cost_per_ex_step)?.into_u64()?;
        let min_marginal_output = cpd.take_field(DATUM_MAPPING.min_marginal_output)?.into_u64()?;
        let output = AssetClass::try_from_pd(cpd.take_field(DATUM_MAPPING.output)?)?;
        let base_price = RelativePrice::try_from_pd(cpd.take_field(DATUM_MAPPING.base_price)?)?;
        let fee = cpd.take_field(DATUM_MAPPING.fee)?.into_u64()?;
        let redeemer_address = PlutusAddress::try_from_pd(cpd.take_field(DATUM_MAPPING.redeemer_address)?)?;
        let cancellation_pkh =
            Ed25519KeyHash::from_raw_bytes(&*cpd.take_field(DATUM_MAPPING.cancellation_pkh)?.into_bytes()?)
                .ok()?;
        let permitted_executors = cpd
            .take_field(DATUM_MAPPING.permitted_executors)?
            .into_vec()?
            .into_iter()
            .filter_map(|pd| Some(Ed25519KeyHash::from_raw_bytes(&*pd.into_bytes()?).ok()?))
            .collect();
        Some(Datum {
            beacon,
            input,
            tradable_input,
            cost_per_ex_step,
            min_marginal_output,
            output,
            base_price,
            fee,
            redeemer_address,
            cancellation_pkh,
            permitted_executors,
        })
    }
}

pub(super) fn beacon_from_oref(
    some_input_oref: OutputRef,
    datum_hash: [u8; 28],
    order_index: u64,
) -> PolicyId {
    let mut bf = vec![];
    bf.append(&mut some_input_oref.tx_hash().to_raw_bytes().to_vec());
    bf.append(&mut some_input_oref.index().to_be_bytes().to_vec());
    bf.append(&mut order_index.to_be_bytes().to_vec());
    bf.append(&mut datum_hash.to_vec());
    blake2b224(&*bf).into()
}

pub(super) const MIN_LOVELACE: u64 = 1_500_000;

pub(super) enum OrderState {
    New,
    Subsequent,
}

pub(super) fn order_state<C>(
    beacon: PolicyId,
    datum: PlutusData,
    beacon_index: usize,
    ctx: &C,
) -> Option<OrderState>
where
    C: Has<ConsumedInputs> + Has<ConsumedIdentifiers<Token>> + Has<OutputRef>,
{
    let order_index = ctx.select::<OutputRef>().index();
    let datum_without_beacon = with_erased_beacon_unsafe(datum, beacon_index);
    let datum_hash = blake2b224(&*datum_without_beacon.to_cbor_bytes());
    let valid_fresh_beacon = || {
        ctx.select::<ConsumedInputs>()
            .0
            .exists(|o| beacon_from_oref(*o, datum_hash, order_index) == beacon)
    };
    let consumed_ids = ctx.select::<ConsumedIdentifiers<Token>>().0;
    let consumed_beacons = consumed_ids.count(|b| b.0 == beacon);
    if consumed_beacons == 1 {
        Some(OrderState::Subsequent)
    } else if valid_fresh_beacon() && consumed_ids.is_empty() {
        Some(OrderState::New)
    } else {
        None
    }
}

impl<C> TryFromLedger<TransactionOutput, C> for LimitOrder
where
    C: Has<OperatorCred>
        + Has<OutputRef>
        + Has<ConsumedIdentifiers<Token>>
        + Has<ConsumedInputs>
        + Has<DeployedScriptInfo<{ LimitOrderV1 as u8 }>>
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
                let min_marginal_output = min(conf.min_marginal_output, base_output);
                let max_execution_steps_possible = base_output.checked_div(min_marginal_output);
                let max_execution_steps_available = execution_budget.checked_div(conf.cost_per_ex_step);
                if let (Some(max_execution_steps_possible), Some(max_execution_steps_available)) =
                    (max_execution_steps_possible, max_execution_steps_available)
                {
                    let sufficient_input = total_input_asset_amount >= conf.tradable_input;
                    let sufficient_execution_budget =
                        max_execution_steps_available >= max_execution_steps_possible;
                    let is_permissionless = conf.permitted_executors.is_empty();
                    let executable = is_permissionless
                        || conf
                            .permitted_executors
                            .contains(&ctx.select::<OperatorCred>().into());
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
                        let script_info = ctx.select::<DeployedScriptInfo<{ LimitOrderV1 as u8 }>>();
                        return Some(LimitOrder {
                            beacon: conf.beacon,
                            input_asset: conf.input,
                            input_amount: conf.tradable_input,
                            output_asset: conf.output,
                            output_amount: value.amount_of(conf.output).unwrap_or(0),
                            base_price: harden_price(conf.base_price, conf.tradable_input),
                            execution_budget,
                            fee_asset: AssetClass::Native,
                            fee: conf.fee,
                            min_marginal_output,
                            max_cost_per_ex_step: conf.cost_per_ex_step,
                            redeemer_address: conf.redeemer_address,
                            cancellation_pkh: conf.cancellation_pkh,
                            requires_executor_sig: !is_permissionless,
                            marginal_cost: script_info.marginal_cost,
                            virgin: matches!(order_state, Some(OrderState::New)),
                        });
                    } else {
                        trace!(
                            "UTxO {}, LimitOrder {} :: sufficient_input: {}, sufficient_execution_budget: {}, sufficient_fee: {}, executable: {}, valid_configuration: {}, is_valid_beacon: {}",
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
        }
        None
    }
}

#[derive(Copy, Clone, Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LimitOrderValidation {
    pub min_cost_per_ex_step: u64,
    pub min_fee_lovelace: Lovelace,
}

#[cfg(test)]
mod tests {
    use cml_chain::address::Address;
    use cml_chain::assets::AssetBundle;
    use cml_chain::plutus::PlutusData;
    use cml_chain::transaction::{ConwayFormatTxOut, DatumOption, TransactionOutput};
    use cml_chain::{PolicyId, Value};
    use cml_core::serialization::{Deserialize, Serialize};
    use cml_crypto::{blake2b224, Ed25519KeyHash, TransactionHash};
    use type_equalities::IsEqual;

    use bloom_offchain::execution_engine::liquidity_book::config::{ExecutionCap, ExecutionConfig};
    use bloom_offchain::execution_engine::liquidity_book::market_taker::MarketTaker;
    use bloom_offchain::execution_engine::liquidity_book::{ExternalLBEvents, LiquidityBook, TLB};
    use spectrum_cardano_lib::ex_units::ExUnits;
    use spectrum_cardano_lib::types::TryFromPData;
    use spectrum_cardano_lib::{AssetName, OutputRef, Token};
    use spectrum_offchain::data::small_vec::SmallVec;
    use spectrum_offchain::display::display_option;
    use spectrum_offchain::domain::Has;
    use spectrum_offchain::ledger::TryFromLedger;
    use spectrum_offchain_cardano::creds::OperatorCred;
    use spectrum_offchain_cardano::data::pair::PairId;
    use spectrum_offchain_cardano::data::pool::AnyPool;
    use spectrum_offchain_cardano::deployment::ProtocolValidator::LimitOrderV1;
    use spectrum_offchain_cardano::deployment::{
        DeployedScriptInfo, DeployedValidators, ProtocolScriptHashes,
    };
    use spectrum_offchain_cardano::handler_context::{
        ConsumedIdentifiers, ConsumedInputs, ProducedIdentifiers,
    };

    use crate::orders::limit::{
        beacon_from_oref, unsafe_update_datum, with_erased_beacon_unsafe, Datum, LimitOrder,
        LimitOrderValidation, DATUM_MAPPING,
    };

    struct Context {
        oref: OutputRef,
        limit_order: DeployedScriptInfo<{ LimitOrderV1 as u8 }>,
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

    impl Has<DeployedScriptInfo<{ LimitOrderV1 as u8 }>> for Context {
        fn select<U: IsEqual<DeployedScriptInfo<{ LimitOrderV1 as u8 }>>>(
            &self,
        ) -> DeployedScriptInfo<{ LimitOrderV1 as u8 }> {
            self.limit_order
        }
    }

    #[test]
    fn beacon_derivation_eqv() {
        const DT: &str = "d8799f4100581c80efdc4308cffb24b7e43f1b7951cd77323583383b7db3feae246c8ed8799f4040ff1a000f42401a000dbba01a00021d06d8799f581ccebbd6a8ca954b7fc7a346d0baed4182e0358059f38065de279fb8224b43617264616e6f20436174ffd8799f1b003134b92f84d8621b016345785d8a0000ff00d8799fd8799f581c74104cd5ca6288c1dd2e22ee5c874fdcfc1b81897462d91153496430ffd8799fd8799fd8799f581cde7866fe5068ebf3c87dcdb568da528da5dcb5f659d9b60010e7450fffffffff581c74104cd5ca6288c1dd2e22ee5c874fdcfc1b81897462d911534964309f581c5cb2c968e5d1c7197a6ce7615967310a375545d9bc65063a964335b2ffff";
        const TX: &str = "a88cbaedbe8d5e9e709cddf24886355e876e7c561100b30533ecc2de79a65aa6";
        const IX: u64 = 0;
        const ORDER_IX: u64 = 0;
        let pd = PlutusData::from_cbor_bytes(&*hex::decode(DT).unwrap()).unwrap();
        let pd_without_beacon = with_erased_beacon_unsafe(pd, DATUM_MAPPING.beacon);
        let datum_hash = blake2b224(&*pd_without_beacon.to_cbor_bytes());
        let oref = OutputRef::new(TransactionHash::from_hex(TX).unwrap(), IX);
        assert_eq!(
            beacon_from_oref(oref, datum_hash, ORDER_IX).to_hex(),
            "43787c201c4cc02ce5e71636fce4ed1dc92429a75c08e1cd9fca707b"
        )
    }

    #[test]
    fn update_order_datum() {
        let mut datum = PlutusData::from_cbor_bytes(&*hex::decode(DATA).unwrap()).unwrap();
        let conf_0 = Datum::try_from_pd(datum.clone()).unwrap();
        let new_ti = 20;
        let new_fee = 50;
        unsafe_update_datum(&mut datum, new_ti, new_fee);
        let conf_1 = Datum::try_from_pd(datum).unwrap();
        assert_eq!(
            Datum {
                tradable_input: new_ti,
                fee: new_fee,
                ..conf_0
            },
            conf_1
        );
    }

    const DATA: &str = "d8799f4100581c0896cb319806556fe598d40dcc625c74fa27d29e19a00188c8f830bdd8799f4040ff1a05f5e1001a0007a1201903e8d8799f581c40079b8ba147fb87a00da10deff7ddd13d64daf48802bb3f82530c3e4a53504c41534854657374ffd8799f011903e8ff1a0007a120d8799fd8799f581cab450d88aab97ff92b1614217e5e34b5710e201da0057d3aab684390ffd8799fd8799fd8799f581c1bc47eaccd81a6a13070fdf67304fc5dc9723d85cff31f0421c53101ffffffff581cab450d88aab97ff92b1614217e5e34b5710e201da0057d3aab68439080ff";

    #[test]
    fn try_read() {
        const TX: &str = "a035c1cb245735680dcb3c46a9a3e692fbf550c8a5d7c4ada1471f97cc92dc55";
        const IX: u64 = 0;
        const ORDER_IX: u64 = 0;
        let oref = OutputRef::new(TransactionHash::from_hex(TX).unwrap(), IX);
        let raw_deployment = std::fs::read_to_string("/Users/oskin/dev/spectrum/spectrum-offchain-multiplatform/bloom-cardano-agent/resources/mainnet.deployment.json").expect("Cannot load deployment file");
        let deployment: DeployedValidators =
            serde_json::from_str(&raw_deployment).expect("Invalid deployment file");
        let scripts = ProtocolScriptHashes::from(&deployment);
        let ctx = Context {
            oref,
            limit_order: scripts.limit_order,
            cred: OperatorCred(Ed25519KeyHash::from([0u8; 28])),
            consumed_inputs: SmallVec::new(vec![oref].into_iter()).into(),
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
        let ord = LimitOrder::try_from_ledger(&bearer, &ctx);
        println!("Order: {}", display_option(&ord));
        println!("P_abs: {}", display_option(&ord.map(|x| x.price())));
        assert!(ord.is_some());
    }

    const ORDER_UTXO: &str = "a300583911464eeee89f05aff787d40045af2a40a83fd96c513197d32fbc54ff02de84e72d9d32321535c7f79824766728c1000a2f050b57c63ab3f0d401821a1360e018a1581c97075bf380e65f3c63fb733267adbb7d42eec574428a754d2abca55ba150436861726c65732074686520436861641a00022269028201d81858ecd8799f4100581c64b18826b8f4e3c6a870c84dcf10370b91f4add2550c92e061db356bd8799f4040ff1a13036eaa1a000dbba0199512d8799f581c97075bf380e65f3c63fb733267adbb7d42eec574428a754d2abca55b50436861726c6573207468652043686164ffd8799f1a000254481aa8d2d897ff1a0001e3eed8799fd8799f581caabb78d6719fa76d47849659dd2a8ad211824b895c0109bef8d1d9a4ffd8799fd8799fd8799f581cde84e72d9d32321535c7f79824766728c1000a2f050b57c63ab3f0d4ffffffff581caabb78d6719fa76d47849659dd2a8ad211824b895c0109bef8d1d9a480ff";

    #[test]
    fn invalid_address() {
        let conf = Datum::try_from_pd(PlutusData::from_cbor_bytes(&*hex::decode(DATUM).unwrap()).unwrap());
        assert!(conf.is_none());
    }

    const DATUM: &str = "d8799f4100581cec1183f8ce49fc76c936af91b7e80f476286f845032766b45f970e46d8799f4040ff1a002dc6c01a000dbba0198de0d8799f581c95a427e384527065f2f8946f5e86320d0117839a5e98ea2c0b55fb004448554e54ffd8799f1a48294eab1b000000174876e800ff00d8799fd8799f581c4c06517294a2785ad3fa59acfa3f2db6c8126d9ebc63d4a7bb304149ffd8799fd8799f581c04eae47e956845ee525c0a8e38d8e5049d906be00c2b497735b9ab8dffffff581c4c06517294a2785ad3fa59acfa3f2db6c8126d9ebc63d4a7bb3041499f581c5cb2c968e5d1c7197a6ce7615967310a375545d9bc65063a964335b2ffff";

    const D0: &str = "d8798c4100581c74e8354f26ed5740fa6c351bcc951f7b40ead8cd9df607345705aa80d8798240401a02160ec01a0007a1201a005b7902d87982581c5ac3d4bdca238105a040a565e5d7e734b7c9e1630aec7650e809e34a46535155495254d879821b002a986523ac68be1b00038d7ea4c6800000d87982d87981581cdaf41ff8f2c73d0ad4ffa7f240f82470d2c254a4e6d62a79ff8c02bfd87981d87981d87981581c77e9da83f52a7579be92be3850554c448eab1b1ca3734ed201b48491581cdaf41ff8f2c73d0ad4ffa7f240f82470d2c254a4e6d62a79ff8c02bf81581c17979109209d255917b8563d1e50a5be8123d5e283fbc6fbb04550c6";
    const D1: &str = "d8799f4100581cfb7be11d69e05140e162a8256eba314c4a7f1b0a70a66df7f11e82b6d8799f581c5ac3d4bdca238105a040a565e5d7e734b7c9e1630aec7650e809e34a46535155495254ff1a062ad83d1a0007a1201a00653c87d8799f4040ffd8799f1a00653c871a062ad83dff00d8799fd8799f581c533540cc9ca1c01b0ef375d4a8beaa4e3c43f5813ea485e4e66f5b53ffd8799fd8799fd8799f581c582e86886fc17df6e1c8f951c1325086713ba8e4e8948f05710947efffffffff581c533540cc9ca1c01b0ef375d4a8beaa4e3c43f5813ea485e4e66f5b539f581c17979109209d255917b8563d1e50a5be8123d5e283fbc6fbb04550c6ffff";

    #[test]
    fn recipe_fill_fragment_from_fragment_batch() {
        const TX: &str = "d90ff0194f2ca8dc832ab0550375ef688a74748d963be33cd08622dd0ba65af6";
        const IX: u64 = 0;
        const ORDER_IX: u64 = 0;
        let oref = OutputRef::new(TransactionHash::from_hex(TX).unwrap(), IX);
        let raw_deployment = std::fs::read_to_string("/Users/oskin/dev/spectrum/spectrum-offchain-multiplatform/bloom-cardano-agent/resources/mainnet.deployment.json").expect("Cannot load deployment file");
        let deployment: DeployedValidators =
            serde_json::from_str(&raw_deployment).expect("Invalid deployment file");
        let scripts = ProtocolScriptHashes::from(&deployment);
        let ctx = Context {
            oref,
            limit_order: scripts.limit_order,
            cred: OperatorCred(
                Ed25519KeyHash::from_hex("17979109209d255917b8563d1e50a5be8123d5e283fbc6fbb04550c6").unwrap(),
            ),
            consumed_inputs: SmallVec::new(vec![].into_iter()).into(),
            consumed_identifiers: Default::default(),
            produced_identifiers: Default::default(),
        };
        let d0 = PlutusData::from_cbor_bytes(&*hex::decode(D0).unwrap()).unwrap();
        let o0 = TransactionOutput::new_conway_format_tx_out(ConwayFormatTxOut {
            address: Address::from_bech32("addr1z8d70g7c58vznyye9guwagdza74x36f3uff0eyk2zwpcpxmha8dg8af2w4umay478pg92nzy3643k89rwd8dyqd5sjgspt95mw").unwrap(),
            amount: Value::new(37000000, AssetBundle::new()),
            datum_option: Some(DatumOption::Datum {
                datum: d0,
                len_encoding: Default::default(),
                tag_encoding: None,
                datum_tag_encoding: None,
                datum_bytes_encoding: Default::default(),
            }),
            script_reference: None,
            encodings: None,
        });
        let d1 = PlutusData::from_cbor_bytes(&*hex::decode(D1).unwrap()).unwrap();
        let mut asset1 = AssetBundle::new();
        asset1.set(
            PolicyId::from_hex("5ac3d4bdca238105a040a565e5d7e734b7c9e1630aec7650e809e34a").unwrap(),
            AssetName::try_from_hex("535155495254").unwrap().into(),
            103471165,
        );
        let o1 = TransactionOutput::new_conway_format_tx_out(ConwayFormatTxOut {
            address: Address::from_bech32("addr1z8d70g7c58vznyye9guwagdza74x36f3uff0eyk2zwpcpx6c96rgsm7p0hmwrj8e28qny5yxwya63e8gjj8s2ugfglhsxedx9j").unwrap(),
            amount: Value::new(3000000, asset1),
            datum_option: Some(DatumOption::Datum {
                datum: d1,
                len_encoding: Default::default(),
                tag_encoding: None,
                datum_tag_encoding: None,
                datum_bytes_encoding: Default::default(),
            }),
            script_reference: None,
            encodings: None,
        });
        dbg!(LimitOrder::try_from_ledger(&o0, &ctx));
        dbg!(LimitOrder::try_from_ledger(&o1, &ctx));
        let mut book = TLB::<LimitOrder, AnyPool, PairId, ExUnits>::new(
            0,
            ExecutionConfig {
                execution_cap: ExecutionCap {
                    soft: ExUnits {
                        mem: 5000000,
                        steps: 4000000000,
                    },
                    hard: ExUnits {
                        mem: 14000000,
                        steps: 10000000000,
                    },
                },
                o2o_allowed: true,
                base_step_budget: 90000.into(),
            },
            PairId::dummy(),
        );
        vec![o0, o1]
            .into_iter()
            .filter_map(|o| LimitOrder::try_from_ledger(&o, &ctx))
            .for_each(|o| book.update_taker(o));
        let recipe = book.attempt();
        dbg!(recipe);
    }
}
