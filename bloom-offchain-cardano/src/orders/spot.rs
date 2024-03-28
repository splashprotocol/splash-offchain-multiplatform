use std::cmp::Ordering;

use cml_chain::builders::tx_builder::TransactionUnspentOutput;
use cml_chain::plutus::{ConstrPlutusData, PlutusData};
use cml_chain::PolicyId;
use cml_crypto::{blake2b224, Ed25519KeyHash, RawBytesEncoding};
use cml_multi_era::babbage::BabbageTransactionOutput;
use log::{info, trace};
use num_rational::Ratio;

use bloom_offchain::execution_engine::liquidity_book::fragment::{Fragment, OrderState, StateTrans};
use bloom_offchain::execution_engine::liquidity_book::side::SideM;
use bloom_offchain::execution_engine::liquidity_book::time::TimeBounds;
use bloom_offchain::execution_engine::liquidity_book::types::{
    AbsolutePrice, ExBudgetUsed, ExCostUnits, ExFeeUsed, FeeAsset, InputAsset, OutputAsset, RelativePrice,
};
use bloom_offchain::execution_engine::liquidity_book::weight::Weighted;
use spectrum_cardano_lib::address::PlutusAddress;
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
use spectrum_offchain_cardano::deployment::{test_address, DeployedScriptHash};
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
}

impl PartialOrd for LimitOrder {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        match self.price().partial_cmp(&other.price()) {
            Some(Ordering::Equal) => self.weight().partial_cmp(&other.weight()),
            cmp => cmp,
        }
    }
}

impl Ord for LimitOrder {
    fn cmp(&self, other: &Self) -> Ordering {
        match self.price().cmp(&other.price()) {
            Ordering::Equal => self.weight().cmp(&other.weight()),
            cmp => cmp,
        }
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
        self.input_amount -= removed_input;
        self.output_amount += added_output;
        let budget_used = self.max_cost_per_ex_step;
        self.execution_budget -= budget_used;
        let fee_used = self.linear_fee(removed_input);
        self.fee -= fee_used;
        let next_st = if self.execution_budget < self.max_cost_per_ex_step || self.input_amount == 0 {
            StateTrans::EOL
        } else {
            StateTrans::Active(self)
        };
        (next_st, budget_used, fee_used)
    }
}

impl Fragment for LimitOrder {
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

    fn weighted_fee(&self) -> FeeAsset<Ratio<u64>> {
        Ratio::new(self.fee, self.input_amount)
    }

    fn marginal_cost_hint(&self) -> ExCostUnits {
        160000000
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

struct DatumNativeToTokenMapping {
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

const DATUM_MAPPING: DatumNativeToTokenMapping = DatumNativeToTokenMapping {
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

pub fn unsafe_update_n2t_variables(
    data: &mut PlutusData,
    tradable_input: InputAsset<u64>,
    fee: FeeAsset<u64>,
) {
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

impl<C> TryFromLedger<BabbageTransactionOutput, C> for LimitOrder
where
    C: Has<OperatorCred> + Has<ConsumedInputs> + Has<DeployedScriptHash<{ LimitOrderV1 as u8 }>>,
{
    fn try_from_ledger(repr: &BabbageTransactionOutput, ctx: &C) -> Option<Self> {
        trace!(target: "offchain", "LimitOrder::try_from_ledger");
        if test_address(repr.address(), ctx) {
            info!(target: "offchain", "LimitOrder address coincides");
            let value = repr.value().clone();
            let conf = Datum::try_from_pd(repr.datum()?.into_pd()?)?;
            let total_ada_input = value.amount_of(AssetClass::Native)?;
            if total_ada_input < conf.tradable_input {
                return None;
            }
            let execution_budget = total_ada_input - conf.tradable_input;
            let is_permissionless = conf.permitted_executors.is_empty();
            if is_permissionless
                || conf
                    .permitted_executors
                    .contains(&ctx.select::<OperatorCred>().into())
            {
                // beacon must be derived from one of consumed utxos.
                let valid_beacon = ctx
                    .select::<ConsumedInputs>()
                    .find(|o| beacon_from_oref(*o) == conf.beacon);
                if valid_beacon && execution_budget > conf.cost_per_ex_step {
                    info!(target: "offchain", "Obtained Spot order from ledger.");
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
                    });
                }
            }
        }

        None
    }
}

/// Reference Script Output for [LimitOrder].
#[derive(Debug, Clone)]
pub struct SpotOrderRefScriptOutput(pub TransactionUnspentOutput);

/// Reference Script Output for batch validator of [LimitOrder].
#[derive(Debug, Clone)]
pub struct SpotOrderBatchValidatorRefScriptOutput(pub TransactionUnspentOutput);

#[cfg(test)]
mod tests {
    use crate::orders::spot::{beacon_from_oref, LimitOrder};
    use cml_core::serialization::Deserialize;
    use cml_crypto::{Ed25519KeyHash, RawBytesEncoding, ScriptHash, TransactionHash};
    use cml_multi_era::babbage::BabbageTransactionOutput;
    use spectrum_cardano_lib::OutputRef;
    use spectrum_offchain::data::Has;
    use spectrum_offchain::ledger::TryFromLedger;
    use spectrum_offchain_cardano::creds::OperatorCred;
    use spectrum_offchain_cardano::deployment::ProtocolValidator::LimitOrderV1;
    use spectrum_offchain_cardano::deployment::{
        DeployedScriptHash, DeployedValidators, ProtocolScriptHashes,
    };
    use spectrum_offchain_cardano::utxo::ConsumedInputs;
    use type_equalities::IsEqual;

    struct Context {
        limit_order: DeployedScriptHash<{ LimitOrderV1 as u8 }>,
        cred: OperatorCred,
        consumed_inputs: ConsumedInputs,
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

    impl Has<DeployedScriptHash<{ LimitOrderV1 as u8 }>> for Context {
        fn select<U: IsEqual<DeployedScriptHash<{ LimitOrderV1 as u8 }>>>(
            &self,
        ) -> DeployedScriptHash<{ LimitOrderV1 as u8 }> {
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
    fn try_read() {
        let sh = ScriptHash::from_raw_bytes(&[
            43, 5, 173, 152, 64, 206, 96, 8, 59, 78, 87, 134, 150, 142, 30, 23, 248, 69, 158, 20, 157, 154,
            250, 196, 212, 77, 255, 23,
        ]);
        println!("{}", sh.unwrap());
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
        LimitOrder::try_from_ledger(&bearer, &ctx).expect("LimitOrder expected");
    }
    const ORDER_UTXO: &str = "a300581d702b05ad9840ce60083b4e5786968e1e17f8459e149d9afac4d44dff1701821a002625a0a1581cfd10da3e6a578708c877e14b6aaeda8dc3a36f666a346eec52a30b3aa14974657374746f6b656e1a0001d4c0028201d81858e1d8799f4100581cc74ecb78de2fb0e4ec31f1c556d22ec088f2ef411299a37d1ede3b33d8799f581cfd10da3e6a578708c877e14b6aaeda8dc3a36f666a346eec52a30b3a4974657374746f6b656eff1a0001d4c01a0007a1201a00989680d8799f4040ffd8799f1903e801ff1a0007a120d8799fd8799f581c4be4fa25f029d14c0d723af4a1e6fa7133fc3a610f880336ad685cbaffd8799fd8799fd8799f581c5bda73043d43ad8df5ce75639cf48e1f2b4545403be92f0113e37537ffffffff581c4be4fa25f029d14c0d723af4a1e6fa7133fc3a610f880336ad685cba80ff";
}
