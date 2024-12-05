use std::cmp::{max, Ordering};
use std::fmt::{Display, Formatter};

use cml_chain::plutus::PlutusData;
use cml_chain::transaction::TransactionOutput;
use cml_chain::PolicyId;
use cml_crypto::{Ed25519KeyHash, RawBytesEncoding};
use derive_more::{From, Into};
use num_rational::Ratio;

use bloom_offchain::execution_engine::liquidity_book::core::{Next, TerminalTake, Unit};
use bloom_offchain::execution_engine::liquidity_book::market_taker::{MarketTaker, TakerBehaviour};
use bloom_offchain::execution_engine::liquidity_book::side::Side;
use bloom_offchain::execution_engine::liquidity_book::time::TimeBounds;
use bloom_offchain::execution_engine::liquidity_book::types::{
    AbsolutePrice, FeeAsset, InputAsset, Lovelace, OutputAsset,
};
use bloom_offchain::execution_engine::liquidity_book::weight::Weighted;
use spectrum_cardano_lib::address::PlutusAddress;
use spectrum_cardano_lib::ex_units::ExUnits;
use spectrum_cardano_lib::plutus_data::{
    ConstrPlutusDataExtension, DatumExtension, IntoPlutusData, PlutusDataExtension,
};
use spectrum_cardano_lib::transaction::TransactionOutputExtension;
use spectrum_cardano_lib::types::TryFromPData;
use spectrum_cardano_lib::value::ValueExtension;
use spectrum_cardano_lib::{AssetClass, AssetName, Token};
use spectrum_offchain::domain::{Has, Stable, Tradable};
use spectrum_offchain::ledger::TryFromLedger;
use spectrum_offchain_cardano::data::pair::{side_of, PairId};
use spectrum_offchain_cardano::deployment::ProtocolValidator::GridOrderNative;
use spectrum_offchain_cardano::deployment::{test_address, DeployedScriptInfo};

use crate::relative_side::RelativeSide;

/// Quote/Base price relative to order.
#[derive(Debug, Copy, Clone, PartialEq, Eq, Into, From)]
pub struct GridPrice(Ratio<u128>);
impl GridPrice {
    #[inline]
    pub fn new_unsafe(numer: u128, denom: u128) -> Self {
        Self(Ratio::new_raw(numer, denom))
    }
    pub fn value(self) -> Ratio<u128> {
        self.0
    }
    pub fn num(&self) -> u128 {
        *self.0.numer()
    }
    pub fn den(&self) -> u128 {
        *self.0.denom()
    }
}

/// Open Grid Order.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct GridOrder {
    pub beacon: PolicyId,
    pub base_asset: AssetClass,
    pub quote_asset: AssetClass,
    pub buy_shift_factor: Ratio<u128>,
    pub sell_shift_factor: Ratio<u128>,
    pub base_reserves: u64,
    pub quote_reserves: u64,
    pub quote_offer: u64,
    pub price: GridPrice,
    /// Side relative to the order .
    /// Note, it may differ from absolute (canonical) Side in TLB.
    pub side: RelativeSide,
    /// Minimal marginal output of base asset allowed per execution step.
    pub min_marginal_output_base: u64,
    /// Minimal marginal output of quote asset allowed per execution step.
    pub min_marginal_output_quote: u64,
    /// Lovelace allowed to be utilized at once to cover TX fee.
    pub max_execution_budget_per_step: Lovelace,
    pub remaining_execution_budget: Lovelace,
    /// Where the output from the order must go.
    pub redeemer_address: PlutusAddress,
    /// How many execution units each order consumes.
    pub marginal_cost: ExUnits,
}

impl GridOrder {
    /// Relative input, output assets of the order.
    pub fn relative_io(&self) -> (AssetClass, AssetClass) {
        match self.side.value() {
            Side::Bid => (self.quote_asset, self.base_asset),
            Side::Ask => (self.base_asset, self.quote_asset),
        }
    }

    /// Canonical input, output assets of the order.
    pub fn absolute_io(&self) -> (AssetClass, AssetClass) {
        let relative_side = self.side.value();
        let absolute_side = self.side();
        if absolute_side == relative_side {
            match relative_side {
                Side::Bid => (self.quote_asset, self.base_asset),
                Side::Ask => (self.base_asset, self.quote_asset),
            }
        } else {
            match relative_side {
                Side::Ask => (self.quote_asset, self.base_asset),
                Side::Bid => (self.base_asset, self.quote_asset),
            }
        }
    }
}

impl Display for GridOrder {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        let (i, o) = self.relative_io();
        f.write_str(
            format!(
                "GridOrder({}, {}, {}, p={}, in={} {}, out={} {}, budget={})",
                self.beacon,
                self.side(),
                self.pair_id(),
                self.price(),
                self.input(),
                i,
                self.output(),
                o,
                self.remaining_execution_budget,
            )
            .as_str(),
        )
    }
}

impl PartialOrd for GridOrder {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for GridOrder {
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

impl TakerBehaviour for GridOrder {
    fn with_updated_time(self, _: u64) -> Next<Self, Unit> {
        Next::Succ(self)
    }

    fn with_applied_trade(
        mut self,
        removed_input: InputAsset<u64>,
        added_output: OutputAsset<u64>,
    ) -> Next<Self, TerminalTake> {
        let relative_side = self.side.value();
        let absolute_side = self.side();
        let mut mock = <u64>::MAX;
        let (input_reserves, input_offer, output_reserves, output_offer) = if relative_side == absolute_side {
            match relative_side {
                Side::Bid => (
                    &mut self.quote_reserves,
                    &mut mock,
                    &mut self.base_reserves,
                    &mut self.quote_offer,
                ),
                Side::Ask => (
                    &mut self.base_reserves,
                    &mut self.quote_offer,
                    &mut self.quote_reserves,
                    &mut mock,
                ),
            }
        } else {
            match relative_side {
                Side::Ask => (
                    &mut self.quote_reserves,
                    &mut mock,
                    &mut self.base_reserves,
                    &mut self.quote_offer,
                ),
                Side::Bid => (
                    &mut self.base_reserves,
                    &mut self.quote_offer,
                    &mut self.quote_reserves,
                    &mut mock,
                ),
            }
        };
        *input_reserves -= removed_input;
        *input_offer -= removed_input;
        *output_reserves += added_output;
        *output_offer += added_output;
        let budget_used = self.max_execution_budget_per_step;
        self.remaining_execution_budget -= budget_used;
        match relative_side {
            Side::Bid if self.quote_reserves == 0 => {
                self.side = Side::Ask.into();
                self.price = Ratio::new_raw(
                    self.price.num() * *self.buy_shift_factor.numer(),
                    self.price.den() * *self.buy_shift_factor.denom(),
                )
                .into();
            }
            Side::Ask if self.quote_offer == 0 => {
                self.side = Side::Bid.into();
                self.price = Ratio::new_raw(
                    self.price.num() * *self.sell_shift_factor.numer(),
                    self.price.den() * *self.sell_shift_factor.denom(),
                )
                .into();
            }
            _ => (),
        }
        Next::Succ(self)
    }

    fn with_budget_corrected(mut self, delta: i64) -> (i64, Self) {
        let budget_remainder = self.remaining_execution_budget as i64;
        let corrected_remainder = budget_remainder + delta;
        let updated_budget_remainder = max(corrected_remainder, 0);
        let real_delta = updated_budget_remainder - budget_remainder;
        self.remaining_execution_budget = updated_budget_remainder as u64;
        (real_delta, self)
    }

    fn with_fee_charged(self, _: u64) -> Self {
        self
    }

    fn with_output_added(mut self, added_output: u64) -> Self {
        let relative_side = self.side.value();
        let absolute_side = self.side();
        let mut mock = <u64>::MAX;
        let (output_reserves, output_offer) = if relative_side == absolute_side {
            match relative_side {
                Side::Bid => (&mut self.base_reserves, &mut self.quote_offer),
                Side::Ask => (&mut self.quote_reserves, &mut mock),
            }
        } else {
            match relative_side {
                Side::Ask => (&mut self.base_reserves, &mut self.quote_offer),
                Side::Bid => (&mut self.quote_reserves, &mut mock),
            }
        };
        *output_reserves += added_output;
        *output_offer += added_output;
        self
    }

    fn try_terminate(self) -> Next<Self, TerminalTake> {
        Next::Succ(self)
    }
}

impl MarketTaker for GridOrder {
    type U = ExUnits;

    fn side(&self) -> Side {
        let (input, output) = self.relative_io();
        side_of(input, output)
    }

    fn input(&self) -> u64 {
        let relative_side = self.side.value();
        match relative_side {
            Side::Bid => self.quote_offer,
            Side::Ask => self.base_reserves,
        }
    }

    fn output(&self) -> OutputAsset<u64> {
        let relative_side = self.side.value();
        match relative_side {
            Side::Ask => self.quote_reserves,
            Side::Bid => self.base_reserves,
        }
    }

    fn price(&self) -> AbsolutePrice {
        let relative_side = self.side.value();
        if relative_side == self.side() {
            self.price.value().into()
        } else {
            self.price.value().pow(-1).into()
        }
    }

    fn operator_fee(&self, _: InputAsset<u64>) -> FeeAsset<u64> {
        0
    }

    fn fee(&self) -> FeeAsset<u64> {
        0
    }

    fn budget(&self) -> FeeAsset<u64> {
        self.max_execution_budget_per_step
    }

    fn consumable_budget(&self) -> FeeAsset<u64> {
        self.max_execution_budget_per_step
    }

    fn marginal_cost_hint(&self) -> ExUnits {
        self.marginal_cost
    }

    fn min_marginal_output(&self) -> OutputAsset<u64> {
        self.min_marginal_output_base
    }

    fn time_bounds(&self) -> TimeBounds<u64> {
        TimeBounds::None
    }
}

impl Stable for GridOrder {
    type StableId = Token;
    fn stable_id(&self) -> Self::StableId {
        Token(self.beacon, AssetName::zero())
    }
    fn is_quasi_permanent(&self) -> bool {
        false
    }
}

impl Tradable for GridOrder {
    type PairId = PairId;

    fn pair_id(&self) -> Self::PairId {
        PairId::canonical(self.quote_asset, self.base_asset)
    }
}

#[derive(Debug)]
struct DatumNative {
    beacon: PolicyId,
    token: AssetClass,
    buy_shift_factor: Ratio<u128>,
    sell_shift_factor: Ratio<u128>,
    max_lovelace_offer: Lovelace,
    lovelace_offer: Lovelace,
    price: GridPrice,
    side: RelativeSide,
    budget_per_transaction: Lovelace,
    min_marginal_output_lovelace: Lovelace,
    min_marginal_output_token: u64,
    redeemer_address: PlutusAddress,
    cancellation_pkh: Ed25519KeyHash,
}

struct DatumNativeMapping {
    beacon: usize,
    token: usize,
    buy_shift_factor: usize,
    sell_shift_factor: usize,
    max_lovelace_offer: usize,
    lovelace_offer: usize,
    price: usize,
    side: usize,
    budget_per_transaction: usize,
    min_marginal_output_lovelace: usize,
    min_marginal_output_token: usize,
    redeemer_address: usize,
    cancellation_pkh: usize,
}

const DATUM_NATIVE_MAPPING: DatumNativeMapping = DatumNativeMapping {
    beacon: 0,
    token: 1,
    buy_shift_factor: 2,
    sell_shift_factor: 3,
    max_lovelace_offer: 4,
    lovelace_offer: 5,
    price: 6,
    side: 7,
    budget_per_transaction: 8,
    min_marginal_output_lovelace: 9,
    min_marginal_output_token: 10,
    redeemer_address: 11,
    cancellation_pkh: 12,
};

impl TryFromPData for DatumNative {
    fn try_from_pd(data: PlutusData) -> Option<Self> {
        let mut cpd = data.into_constr_pd()?;
        Some(DatumNative {
            beacon: PolicyId::from_raw_bytes(&*cpd.take_field(DATUM_NATIVE_MAPPING.beacon)?.into_bytes()?)
                .ok()?,
            token: AssetClass::try_from_pd(cpd.take_field(DATUM_NATIVE_MAPPING.token)?)?,
            buy_shift_factor: <Ratio<u128>>::try_from_pd(
                cpd.take_field(DATUM_NATIVE_MAPPING.buy_shift_factor)?,
            )?,
            sell_shift_factor: <Ratio<u128>>::try_from_pd(
                cpd.take_field(DATUM_NATIVE_MAPPING.sell_shift_factor)?,
            )?,
            max_lovelace_offer: cpd
                .take_field(DATUM_NATIVE_MAPPING.max_lovelace_offer)?
                .into_u64()?,
            lovelace_offer: cpd.take_field(DATUM_NATIVE_MAPPING.lovelace_offer)?.into_u64()?,
            price: <Ratio<u128>>::try_from_pd(cpd.take_field(DATUM_NATIVE_MAPPING.price)?)?.into(),
            side: bool::try_from_pd(cpd.take_field(DATUM_NATIVE_MAPPING.side)?).map(|flag| {
                if flag {
                    Side::Bid.into()
                } else {
                    Side::Ask.into()
                }
            })?,
            budget_per_transaction: cpd
                .take_field(DATUM_NATIVE_MAPPING.budget_per_transaction)?
                .into_u64()?,
            min_marginal_output_lovelace: cpd
                .take_field(DATUM_NATIVE_MAPPING.min_marginal_output_lovelace)?
                .into_u64()?,
            min_marginal_output_token: cpd
                .take_field(DATUM_NATIVE_MAPPING.min_marginal_output_token)?
                .into_u64()?,
            redeemer_address: PlutusAddress::try_from_pd(
                cpd.take_field(DATUM_NATIVE_MAPPING.redeemer_address)?,
            )?,
            cancellation_pkh: Ed25519KeyHash::from_raw_bytes(
                &*cpd
                    .take_field(DATUM_NATIVE_MAPPING.cancellation_pkh)?
                    .into_bytes()?,
            )
            .ok()?,
        })
    }
}

pub fn unsafe_update_datum(
    data: &mut PlutusData,
    lovelace_offer: u64,
    price: GridPrice,
    relative_side: RelativeSide,
) {
    let cpd = data.get_constr_pd_mut().unwrap();
    cpd.set_field(DATUM_NATIVE_MAPPING.lovelace_offer, lovelace_offer.into_pd());
    cpd.set_field(DATUM_NATIVE_MAPPING.price, price.value().into_pd());
    cpd.set_field(DATUM_NATIVE_MAPPING.side, relative_side.into_pd());
}

impl<C> TryFromLedger<TransactionOutput, C> for GridOrder
where
    C: Has<DeployedScriptInfo<{ GridOrderNative as u8 }>>,
{
    fn try_from_ledger(repr: &TransactionOutput, ctx: &C) -> Option<Self> {
        if test_address(repr.address(), ctx) {
            let value = repr.value().clone();
            let conf = DatumNative::try_from_pd(repr.datum()?.into_pd()?)?;
            let base = conf.token;
            let total_lovelace = value.amount_of(AssetClass::Native)?;
            let total_base = value.amount_of(base).unwrap_or(0);
            return with_consistency_verified_native(Self {
                beacon: conf.beacon,
                base_asset: base,
                quote_asset: AssetClass::Native,
                buy_shift_factor: conf.buy_shift_factor,
                sell_shift_factor: conf.sell_shift_factor,
                base_reserves: total_base,
                quote_reserves: total_lovelace,
                quote_offer: conf.lovelace_offer,
                price: harden_price(conf.price, conf.side),
                side: conf.side,
                min_marginal_output_base: conf.min_marginal_output_token,
                min_marginal_output_quote: conf.min_marginal_output_lovelace,
                max_execution_budget_per_step: conf.budget_per_transaction,
                remaining_execution_budget: conf.budget_per_transaction,
                redeemer_address: conf.redeemer_address,
                marginal_cost: ctx.get().marginal_cost,
            });
        }
        None
    }
}

fn with_consistency_verified_native(grid_order: GridOrder) -> Option<GridOrder> {
    let min_real_lovelace = if matches!(grid_order.side.value(), Side::Bid) {
        grid_order.quote_offer + grid_order.remaining_execution_budget
    } else {
        grid_order.remaining_execution_budget
    };
    let consistent_value = grid_order.quote_reserves >= min_real_lovelace;
    if consistent_value {
        return Some(grid_order);
    }
    None
}

fn harden_price(p: GridPrice, side: RelativeSide) -> GridPrice {
    match side.value() {
        Side::Bid => GridPrice::new_unsafe(p.num(), p.den() + 1),
        Side::Ask => GridPrice::new_unsafe(p.num() + 1, p.den()),
    }
}

#[cfg(test)]
mod tests {
    use cml_chain::plutus::PlutusData;
    use cml_chain::transaction::TransactionOutput;
    use cml_core::serialization::Deserialize;
    use cml_multi_era::babbage::BabbageTransactionOutput;
    use type_equalities::IsEqual;

    use bloom_offchain::execution_engine::liquidity_book::linear_output_unsafe;
    use bloom_offchain::execution_engine::liquidity_book::market_taker::MarketTaker;
    use bloom_offchain::execution_engine::liquidity_book::side::Side;
    use spectrum_cardano_lib::ex_units::ExUnits;
    use spectrum_cardano_lib::types::TryFromPData;
    use spectrum_cardano_lib::AssetClass;
    use spectrum_offchain::domain::Has;
    use spectrum_offchain::ledger::TryFromLedger;
    use spectrum_offchain_cardano::deployment::ProtocolValidator::GridOrderNative;
    use spectrum_offchain_cardano::deployment::{
        DeployedScriptInfo, DeployedValidators, ProtocolScriptHashes,
    };

    use crate::orders::grid::{unsafe_update_datum, DatumNative, GridOrder, GridPrice};

    #[test]
    fn read_datum() {
        let datum = PlutusData::from_cbor_bytes(&*hex::decode(DATUM).unwrap()).unwrap();
        let order_state = DatumNative::try_from_pd(datum).unwrap();
        dbg!(order_state);
    }

    #[test]
    fn update_datum() {
        let mut datum = PlutusData::from_cbor_bytes(&*hex::decode(DATUM).unwrap()).unwrap();
        let order_state_0 = DatumNative::try_from_pd(datum.clone()).unwrap();
        unsafe_update_datum(&mut datum, 100, GridPrice::new_unsafe(1, 2), Side::Ask.into());
        let order_state_1 = DatumNative::try_from_pd(datum.clone()).unwrap();
        dbg!(order_state_0);
        dbg!(order_state_1);
    }

    #[test]
    fn correct_tlb_interface() {
        let mut datum = PlutusData::from_cbor_bytes(&*hex::decode(DATUM).unwrap()).unwrap();
        let order_state = DatumNative::try_from_pd(datum.clone()).unwrap();
        let order = GridOrder {
            beacon: order_state.beacon,
            base_asset: order_state.token,
            quote_asset: AssetClass::Native,
            buy_shift_factor: order_state.buy_shift_factor,
            sell_shift_factor: order_state.sell_shift_factor,
            base_reserves: 0,
            quote_reserves: 120_000_000,
            quote_offer: 100_000_000,
            price: order_state.price,
            side: order_state.side,
            min_marginal_output_base: order_state.min_marginal_output_token,
            min_marginal_output_quote: order_state.min_marginal_output_lovelace,
            max_execution_budget_per_step: order_state.budget_per_transaction,
            remaining_execution_budget: order_state.budget_per_transaction,
            redeemer_address: order_state.redeemer_address,
            marginal_cost: ExUnits { mem: 0, steps: 0 },
        };
        assert_eq!(order.input(), order.quote_offer);
        assert_eq!(order.output(), order.base_reserves);
        assert_eq!(order.side(), !order.side.value());
        assert_eq!(order.price(), order.price.value().pow(-1).into());
        assert_eq!(
            linear_output_unsafe(order.input(), order.side().wrap(order.price())),
            linear_output_unsafe(
                order.quote_offer,
                order.side.value().wrap(order.price.value().into())
            )
        );
    }

    struct Context {
        grid_order: DeployedScriptInfo<{ GridOrderNative as u8 }>,
    }

    impl Has<DeployedScriptInfo<{ GridOrderNative as u8 }>> for Context {
        fn select<U: IsEqual<DeployedScriptInfo<{ GridOrderNative as u8 }>>>(
            &self,
        ) -> DeployedScriptInfo<{ GridOrderNative as u8 }> {
            self.grid_order
        }
    }

    #[test]
    fn try_read() {
        let raw_deployment = std::fs::read_to_string("/Users/oskin/dev/spectrum/spectrum-offchain-multiplatform/bloom-cardano-agent/resources/mainnet.deployment.json").expect("Cannot load deployment file");
        let deployment: DeployedValidators =
            serde_json::from_str(&raw_deployment).expect("Invalid deployment file");
        let scripts = ProtocolScriptHashes::from(&deployment);
        let ctx = Context {
            grid_order: scripts.grid_order_native,
        };
        let bearer = TransactionOutput::from_cbor_bytes(&*hex::decode(UTXO).unwrap()).unwrap();
        let ord = GridOrder::try_from_ledger(&bearer, &ctx).unwrap();
        println!("Order: {:?}", ord);
        println!("P_abs: {}", ord.price());
    }

    const DATUM: &str = "d8799f581c062221778dde04f0b931f1ae4d74aa746f26deeb464251568c435d26d8799f581ce52964af4fffdb54504859875b1827b60ba679074996156461143dc1454f5054494dffd8799f1903ed1903e8ffd8799f1903e81903e3ff1a01312d001a01312d00d8799f182b1864ffd87a801a0007a1201a004c4b401a00989680d8799fd8799f581c719bee424a97b58b3dca88fe5da6feac6494aa7226f975f3506c5b25ffd8799fd8799fd8799f581c7846f6bb07f5b2825885e4502679e699b4e60a0c4609a46bc35454cdffffffff581c719bee424a97b58b3dca88fe5da6feac6494aa7226f975f3506c5b25ff";
    const UTXO: &str = "a300581d716eff899ca605c05c115f0d7b0d0397e2dd886cd366d77bcb4ac6592201821a01c9c380a1581c062221778dde04f0b931f1ae4d74aa746f26deeb464251568c435d26a14001028201d81858f0d8799f581c062221778dde04f0b931f1ae4d74aa746f26deeb464251568c435d26d8799f581ce52964af4fffdb54504859875b1827b60ba679074996156461143dc1454f5054494dffd8799f1903ed1903e8ffd8799f1903e81903e3ff1a01312d001a01312d00d8799f182b1864ffd87a801a0007a1201a004c4b401a00989680d8799fd8799f581c719bee424a97b58b3dca88fe5da6feac6494aa7226f975f3506c5b25ffd8799fd8799fd8799f581c7846f6bb07f5b2825885e4502679e699b4e60a0c4609a46bc35454cdffffffff581c719bee424a97b58b3dca88fe5da6feac6494aa7226f975f3506c5b25ff";
}
