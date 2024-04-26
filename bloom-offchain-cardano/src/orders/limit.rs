use std::cmp::Ordering;
use std::fmt::{Display, Formatter};

use cml_chain::plutus::{ConstrPlutusData, PlutusData};
use cml_chain::PolicyId;
use cml_crypto::{blake2b224, Ed25519KeyHash, RawBytesEncoding};
use cml_multi_era::babbage::BabbageTransactionOutput;
use log::info;

use bloom_offchain::execution_engine::liquidity_book::fragment::{Fragment, OrderState, StateTrans};
use bloom_offchain::execution_engine::liquidity_book::side::SideM;
use bloom_offchain::execution_engine::liquidity_book::time::TimeBounds;
use bloom_offchain::execution_engine::liquidity_book::types::{
    AbsolutePrice, ExBudgetUsed, ExFeeUsed, FeeAsset, InputAsset, OutputAsset, RelativePrice,
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
use spectrum_cardano_lib::{AssetClass, OutputRef};
use spectrum_offchain::data::{Has, Stable, Tradable};
use spectrum_offchain::ledger::TryFromLedger;
use spectrum_offchain_cardano::creds::OperatorCred;
use spectrum_offchain_cardano::data::pair::{side_of, PairId};
use spectrum_offchain_cardano::deployment::ProtocolValidator::LimitOrderV1;
use spectrum_offchain_cardano::deployment::{test_address, DeployedScriptInfo};
use spectrum_offchain_cardano::utxo::ConsumedInputs;

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
    /// Whether the order has just been created.
    pub virgin: bool,
    /// How many execution units each order consumes.
    pub marginal_cost: ExUnits,
}

impl Display for LimitOrder {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(
            format!(
                "LimOrd({}, {}, {}, in: {} @ {}, out: {})",
                self.beacon,
                self.side(),
                self.pair_id(),
                self.input_amount,
                self.price(),
                self.output_amount
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
        let cmp_by_price = if matches!(self.side(), SideM::Bid) {
            cmp_by_price.reverse()
        } else {
            cmp_by_price
        };
        cmp_by_price
            .then(self.weight().cmp(&other.weight()))
            .then(self.stable_id().cmp(&other.stable_id()))
    }
}

impl OrderState for LimitOrder {
    fn with_updated_time(self, _time: u64) -> StateTrans<Self> {
        StateTrans::Active(self)
    }

    fn with_applied_swap(
        mut self,
        removed_input: u64,
        added_output: u64,
    ) -> (StateTrans<Self>, ExBudgetUsed, ExFeeUsed) {
        let fee_used = self.linear_fee(removed_input);
        self.fee -= fee_used;
        self.input_amount -= removed_input;
        self.output_amount += added_output;
        let budget_used = self.max_cost_per_ex_step;
        self.execution_budget -= budget_used;
        let next_st = if self.execution_budget < self.max_cost_per_ex_step || self.input_amount == 0 {
            StateTrans::EOL
        } else {
            StateTrans::Active(self)
        };
        (next_st, budget_used, fee_used)
    }
}

impl Fragment for LimitOrder {
    type U = ExUnits;

    fn side(&self) -> SideM {
        side_of(self.input_asset, self.output_asset)
    }

    fn input(&self) -> u64 {
        self.input_amount
    }

    fn price(&self) -> AbsolutePrice {
        AbsolutePrice::from_price(self.side(), self.base_price)
    }

    fn linear_fee(&self, input_consumed: InputAsset<u64>) -> FeeAsset<u64> {
        if self.input_amount > 0 {
            self.fee * input_consumed / self.input_amount
        } else {
            0
        }
    }

    fn fee(&self) -> FeeAsset<u64> {
        self.fee
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
    type StableId = PolicyId;
    fn stable_id(&self) -> Self::StableId {
        self.beacon
    }
    fn is_quasi_permanent(&self) -> bool {
        false
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

fn beacon_from_oref(oref: OutputRef) -> PolicyId {
    let mut bf = vec![];
    bf.append(&mut oref.tx_hash().to_raw_bytes().to_vec());
    bf.append(&mut oref.index().to_string().as_bytes().to_vec());
    blake2b224(&*bf).into()
}

const MIN_LOVELACE: u64 = 1_500_000;

impl<C> TryFromLedger<BabbageTransactionOutput, C> for LimitOrder
where
    C: Has<OperatorCred>
        + Has<ConsumedInputs>
        + Has<DeployedScriptInfo<{ LimitOrderV1 as u8 }>>
        + Has<LimitOrderBounds>,
{
    fn try_from_ledger(repr: &BabbageTransactionOutput, ctx: &C) -> Option<Self> {
        if test_address(repr.address(), ctx) {
            let value = repr.value().clone();
            let conf = Datum::try_from_pd(repr.datum()?.into_pd()?)?;
            let total_ada_input = value.amount_of(AssetClass::Native)?;
            let (reserved_lovelace, tradable_lovelace) = match (conf.input, conf.output) {
                (AssetClass::Native, _) => (MIN_LOVELACE, conf.tradable_input),
                (_, AssetClass::Native) => (0, 0),
                _ => (MIN_LOVELACE, 0),
            };
            let execution_budget = total_ada_input
                .checked_sub(reserved_lovelace)
                .and_then(|lov| lov.checked_sub(tradable_lovelace))?;
            let min_output = conf.tradable_input as u128 * conf.base_price.numer() / conf.base_price.denom();
            let min_marginal_output = conf.min_marginal_output as u128;
            let is_permissionless = conf.permitted_executors.is_empty();
            if is_permissionless
                || conf
                    .permitted_executors
                    .contains(&ctx.select::<OperatorCred>().into())
            {
                let bounds = ctx.select::<LimitOrderBounds>();
                let valid_configuration = conf.cost_per_ex_step >= bounds.min_cost_per_ex_step
                    && execution_budget >= conf.cost_per_ex_step
                    && min_output >= min_marginal_output;
                if valid_configuration {
                    // Fresh beacon must be derived from one of consumed utxos.
                    let valid_fresh_beacon = ctx
                        .select::<ConsumedInputs>()
                        .find(|o| beacon_from_oref(*o) == conf.beacon);
                    info!(target: "offchain", "Obtained Limit Order from confirmed UTXO. Beacon: {}", conf.beacon);
                    let script_info = ctx.select::<DeployedScriptInfo<{ LimitOrderV1 as u8 }>>();
                    return Some(LimitOrder {
                        beacon: conf.beacon,
                        input_asset: conf.input,
                        input_amount: conf.tradable_input,
                        output_asset: conf.output,
                        output_amount: value.amount_of(conf.output).unwrap_or(0),
                        base_price: conf.base_price,
                        execution_budget,
                        fee_asset: AssetClass::Native,
                        fee: conf.fee,
                        min_marginal_output: conf.min_marginal_output,
                        max_cost_per_ex_step: conf.cost_per_ex_step,
                        redeemer_address: conf.redeemer_address,
                        cancellation_pkh: conf.cancellation_pkh,
                        requires_executor_sig: !is_permissionless,
                        virgin: valid_fresh_beacon,
                        marginal_cost: script_info.marginal_cost,
                    });
                }
            }
        }
        None
    }
}

#[derive(Copy, Clone, Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LimitOrderBounds {
    pub min_cost_per_ex_step: u64,
}

#[cfg(test)]
mod tests {
    use cml_chain::plutus::PlutusData;
    use cml_core::serialization::Deserialize;
    use cml_crypto::{Ed25519KeyHash, TransactionHash};
    use cml_multi_era::babbage::BabbageTransactionOutput;
    use type_equalities::IsEqual;

    use bloom_offchain::execution_engine::liquidity_book::fragment::Fragment;
    use spectrum_cardano_lib::types::TryFromPData;
    use spectrum_cardano_lib::OutputRef;
    use spectrum_offchain::data::Has;
    use spectrum_offchain::ledger::TryFromLedger;
    use spectrum_offchain_cardano::creds::OperatorCred;
    use spectrum_offchain_cardano::deployment::ProtocolValidator::LimitOrderV1;
    use spectrum_offchain_cardano::deployment::{
        DeployedScriptInfo, DeployedValidators, ProtocolScriptHashes,
    };
    use spectrum_offchain_cardano::utxo::ConsumedInputs;

    use crate::orders::limit::{beacon_from_oref, unsafe_update_datum, Datum, LimitOrder, LimitOrderBounds};

    struct Context {
        limit_order: DeployedScriptInfo<{ LimitOrderV1 as u8 }>,
        cred: OperatorCred,
        consumed_inputs: ConsumedInputs,
    }

    impl Has<LimitOrderBounds> for Context {
        fn select<U: IsEqual<LimitOrderBounds>>(&self) -> LimitOrderBounds {
            LimitOrderBounds {
                min_cost_per_ex_step: 0,
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
        let oref = OutputRef::new(TransactionHash::from_hex(TX).unwrap(), IX);
        assert_eq!(
            beacon_from_oref(oref).to_hex(),
            "eb9575d907ac66f8f0c75c44ad51189a4b41756e8543cd59e331bc02"
        )
    }

    const TX: &str = "6c038a69587061acd5611507e68b1fd3a7e7d189367b7853f3bb5079a118b880";
    const IX: u64 = 1;

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
        let raw_deployment = std::fs::read_to_string("/Users/oskin/dev/spectrum/spectrum-offchain-multiplatform/bloom-cardano-agent/resources/preprod.deployment.json").expect("Cannot load deployment file");
        let deployment: DeployedValidators =
            serde_json::from_str(&raw_deployment).expect("Invalid deployment file");
        let scripts = ProtocolScriptHashes::from(&deployment);
        let ctx = Context {
            limit_order: scripts.limit_order,
            cred: OperatorCred(Ed25519KeyHash::from([0u8; 28])),
            consumed_inputs: ConsumedInputs::new(vec![].into_iter()),
        };
        let bearer = BabbageTransactionOutput::from_cbor_bytes(&*hex::decode(ORDER_UTXO).unwrap()).unwrap();
        let ord = LimitOrder::try_from_ledger(&bearer, &ctx).expect("LimitOrder expected");
        println!("Order: {:?}", ord);
        println!("P_abs: {}", ord.price());
    }

    const ORDER_UTXO: &str = "a300583910dfaa80c9732ed3b7752ba189786723c6709e2876a024f8f4d9910fb36b6723106d7725d57913612286514abb81148d344b1675df297ee22401821a002625a0a1581c4b3459fd18a1dbabe207cd19c9951a9fac9f5c0f9c384e3d97efba26a14574657374431a00b71b00028201d81858e2d8798c4100581c64699bc6a6fb8a1cc7870dce35d3ab73f7fed5e7c385019373b695bbd87982581c4b3459fd18a1dbabe207cd19c9951a9fac9f5c0f9c384e3d97efba264574657374431a00b71b001a0007a1201a0007a120d879824040d879821b00244af2e97c32301b002386f26fc100001a0007a120d87982d87981581c1e5b525041f0d70ad830f1d7dbd2ed7012c1d89788b4385d7bdd0c37d87981d87981d87981581c6b6723106d7725d57913612286514abb81148d344b1675df297ee224581c1e5b525041f0d70ad830f1d7dbd2ed7012c1d89788b4385d7bdd0c3780";

    #[test]
    fn read_config() {
        let conf =
            Datum::try_from_pd(PlutusData::from_cbor_bytes(&*hex::decode(DATUM).unwrap()).unwrap()).unwrap();
        dbg!(conf);
    }

    const DATUM: &str = "d8799f4100581c6386211b2897ed9515fcd840a936d6069d47a35add1ec1cf861da6ccd8799f4040ff1a002dc6c01a00061a801a00061a80d8799f581c4b3459fd18a1dbabe207cd19c9951a9fac9f5c0f9c384e3d97efba26457465737442ffd8799f0101ff1a0007a120d8799fd8799f581c1e5b525041f0d70ad830f1d7dbd2ed7012c1d89788b4385d7bdd0c37ffd8799fd8799fd8799f581c6b6723106d7725d57913612286514abb81148d344b1675df297ee224ffffffff581c1e5b525041f0d70ad830f1d7dbd2ed7012c1d89788b4385d7bdd0c3780ff";
}
