use crate::orders::limit::{BeaconMode, LimitOrder, LimitOrderValidation};
use bloom_offchain::execution_engine::liquidity_book::core::{Next, TerminalTake, Unit};
use bloom_offchain::execution_engine::liquidity_book::market_taker::{MarketTaker, TakerBehaviour};
use bloom_offchain::execution_engine::liquidity_book::side::Side;
use bloom_offchain::execution_engine::liquidity_book::time::TimeBounds;
use bloom_offchain::execution_engine::liquidity_book::types::{
    AbsolutePrice, FeeAsset, InputAsset, Lovelace, OutputAsset, RelativePrice,
};
use bounded_integer::BoundedU64;
use cml_chain::auxdata::Metadata;
use cml_chain::transaction::TransactionOutput;
use cml_chain::PolicyId;
use cml_core::serialization::RawBytesEncoding;
use cml_crypto::{blake2b224, Ed25519Signature};
use log::{info, trace};
use spectrum_cardano_lib::ex_units::ExUnits;
use spectrum_cardano_lib::{AssetClass, OutputRef, Token};
use spectrum_offchain::domain::{Has, SeqState, Stable, Tradable};
use spectrum_offchain::ledger::TryFromLedger;
use spectrum_offchain_cardano::creds::OperatorCred;
use spectrum_offchain_cardano::deployment::DeployedScriptInfo;
use spectrum_offchain_cardano::deployment::ProtocolValidator::LimitOrderV1;
use spectrum_offchain_cardano::handler_context::{
    AuthVerificationKey, ConsumedIdentifiers, ConsumedInputs, ProducedIdentifiers,
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
pub struct AdhocOrder(pub(crate) LimitOrder, /*adhoc_fee_input*/ pub(crate) u64);

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

pub fn beacon_from_oref(
    input_oref: OutputRef,
    order_index: u64,
    input_amount: OutputAsset<u64>,
    input_asset: AssetClass,
    output_asset: AssetClass,
) -> PolicyId {
    let mut bf = vec![];
    bf.append(&mut input_oref.tx_hash().to_raw_bytes().to_vec());
    bf.append(&mut input_oref.index().to_be_bytes().to_vec());
    bf.append(&mut order_index.to_be_bytes().to_vec());
    bf.append(&mut input_amount.to_be_bytes().to_vec());
    bf.append(&mut input_asset.to_bytes());
    bf.append(&mut output_asset.to_bytes());
    blake2b224(&*bf).into()
}

fn is_valid_beacon<C>(
    beacon: PolicyId,
    input_amount: InputAsset<u64>,
    input_asset: AssetClass,
    output_asset: AssetClass,
    ctx: &C,
) -> bool
where
    C: Has<ConsumedInputs>
        + Has<ConsumedIdentifiers<Token>>
        + Has<ProducedIdentifiers<Token>>
        + Has<OutputRef>,
{
    let order_index = ctx.select::<OutputRef>().index();
    let valid_fresh_beacon = || {
        ctx.select::<ConsumedInputs>()
            .0
            .exists(|o| beacon_from_oref(*o, order_index, input_amount, input_asset, output_asset) == beacon)
    };
    let consumed_ids = ctx.select::<ConsumedIdentifiers<Token>>().0;
    let consumed_beacons = consumed_ids.count(|b| b.0 == beacon);
    let produced_beacons = ctx
        .select::<ProducedIdentifiers<Token>>()
        .0
        .count(|b| b.0 == beacon);
    consumed_beacons == 1 && produced_beacons == 1 || valid_fresh_beacon() && consumed_ids.is_empty()
}

pub fn check_auth<C>(beacon: PolicyId, ctx: &C) -> bool
where
    C: Has<Option<Metadata>> + Has<AuthVerificationKey>,
{
    if let Some(signature) = ctx.select::<Option<Metadata>>().and_then(|md| {
        // Signature split into several parts
        md.get(AUTH_MD_KEY)
            .and_then(|d| d.as_list())
            .and_then(|signature_parts| {
                let mut signature = vec![];
                signature_parts.iter().for_each(|entry| {
                    if let Some(bytes_to_add) = (*entry).as_bytes() {
                        let mut bytes_t = bytes_to_add.clone();
                        signature.append(&mut bytes_t)
                    }
                });
                Some(signature)
            })
            .and_then(|raw_sig| Ed25519Signature::from_raw_bytes(&raw_sig).ok())
    }) {
        return ctx
            .select::<AuthVerificationKey>()
            .get_verification_key()
            .verify(beacon.to_raw_bytes(), &signature);
    }
    false
}

const AUTH_MD_KEY: u64 = 7;

impl<C> TryFromLedger<TransactionOutput, C> for AdhocOrder
where
    C: Has<OperatorCred>
        + Has<OutputRef>
        + Has<ConsumedIdentifiers<Token>>
        + Has<ProducedIdentifiers<Token>>
        + Has<ConsumedInputs>
        + Has<DeployedScriptInfo<{ LimitOrderV1 as u8 }>>
        + Has<LimitOrderValidation>
        + Has<BeaconMode>
        + Has<AdhocFeeStructure>
        + Has<Option<Metadata>>
        + Has<AuthVerificationKey>,
{
    fn try_from_ledger(repr: &TransactionOutput, ctx: &C) -> Option<Self> {
        LimitOrder::try_from_ledger(repr, ctx).and_then(|lo| {
            let virtual_input_amount = match (lo.input_asset, lo.output_asset) {
                (AssetClass::Native, _) => Some(subtract_adhoc_fee(lo.input_amount, ctx.get())),
                (_, AssetClass::Native) => Some(lo.input_amount),
                _ => None,
            }?;
            let adhoc_fee_input = lo.input_amount.checked_sub(virtual_input_amount)?;
            let has_stake_part = lo.redeemer_address.stake_cred.is_some();
            let is_valid_beacon =
                is_valid_beacon(lo.beacon, lo.input_amount, lo.input_asset, lo.output_asset, ctx);
            let is_valid_auth = check_auth(lo.beacon, ctx);
            if has_stake_part && is_valid_beacon && is_valid_auth {
                Some(Self(
                    LimitOrder {
                        beacon: lo.beacon,
                        input_asset: lo.input_asset,
                        input_amount: virtual_input_amount,
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
                        marginal_cost: lo.marginal_cost,
                        virgin: lo.virgin,
                    },
                    adhoc_fee_input,
                ))
            } else {
                trace!(
                    "UTxO {}, AdhocOrder {} :: has_stake_part: {}, is_valid_beacon: {}, is_valid_auth: {}",
                    ctx.select::<OutputRef>(),
                    lo.beacon,
                    has_stake_part,
                    is_valid_beacon,
                    is_valid_auth
                );
                None
            }
        })
    }
}
