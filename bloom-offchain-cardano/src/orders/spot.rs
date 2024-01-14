use std::cmp::Ordering;

use cml_chain::certs::Credential;
use cml_chain::plutus::{ConstrPlutusData, PlutusData};
use cml_chain::utils::BigInt;
use cml_chain::PolicyId;
use cml_core::serialization::{LenEncoding, StringEncoding};
use cml_crypto::Ed25519KeyHash;
use cml_multi_era::babbage::BabbageTransactionOutput;

use bloom_offchain::execution_engine::liquidity_book::fragment::{Fragment, OrderState, StateTrans};
use bloom_offchain::execution_engine::liquidity_book::side::SideM;
use bloom_offchain::execution_engine::liquidity_book::time::TimeBounds;
use bloom_offchain::execution_engine::liquidity_book::types::{
    AbsolutePrice, ExecutionCost, FeePerOutput, RelativePrice,
};
use bloom_offchain::execution_engine::liquidity_book::weight::Weighted;
use spectrum_cardano_lib::plutus_data::{ConstrPlutusDataExtension, DatumExtension, PlutusDataExtension};
use spectrum_cardano_lib::transaction::TransactionOutputExtension;
use spectrum_cardano_lib::types::TryFromPData;
use spectrum_cardano_lib::value::ValueExtension;
use spectrum_cardano_lib::{AssetClass, OutputRef};
use spectrum_offchain::data::{Stable, Tradable};
use spectrum_offchain::ledger::TryFromLedger;
use spectrum_offchain_cardano::data::pair::{side_of, PairId};
use spectrum_offchain_cardano::data::ExecutorFeePerToken;

/// Spot order. Can be executed at a configured or better price as long as there is enough budget.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct SpotOrder {
    /// Identifier of the order.
    pub beacon: PolicyId,
    /// What user pays.
    pub input_asset: AssetClass,
    /// Remaining input.
    pub input_amount: u64,
    /// What user receives.
    pub output_asset: AssetClass,
    /// Accumulated output.
    pub output_amount: u64,
    /// Amount of output sufficient for the order to be terminated.
    pub termination_threshold: u64,
    /// Worst acceptable price (Output/Input).
    pub base_price: RelativePrice,
    /// Fee per output unit.
    pub fee_per_output: ExecutorFeePerToken,
    /// Redeemer PKH.
    pub redeemer_pkh: Ed25519KeyHash,
    /// Redeemer stake PKH.
    pub redeemer_stake_pkh: Option<Ed25519KeyHash>,
}

impl SpotOrder {
    pub fn redeemer_cred(&self) -> Credential {
        Credential::PubKey {
            hash: self.redeemer_pkh,
            len_encoding: LenEncoding::Canonical,
            tag_encoding: None,
            hash_encoding: StringEncoding::Canonical,
        }
    }
}

pub fn spot_exec_redeemer(successor_ix: u16) -> PlutusData {
    PlutusData::ConstrPlutusData(ConstrPlutusData::new(
        0,
        vec![PlutusData::Integer(BigInt::from(successor_ix))],
    ))
}

impl PartialOrd for SpotOrder {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        match self.price().partial_cmp(&other.price()) {
            Some(Ordering::Equal) => self.weight().partial_cmp(&other.weight()),
            cmp => cmp,
        }
    }
}

impl Ord for SpotOrder {
    fn cmp(&self, other: &Self) -> Ordering {
        match self.price().cmp(&other.price()) {
            Ordering::Equal => self.weight().cmp(&other.weight()),
            cmp => cmp,
        }
    }
}

impl OrderState for SpotOrder {
    fn with_updated_time(self, _time: u64) -> StateTrans<Self> {
        StateTrans::Active(self)
    }

    fn with_updated_liquidity(mut self, removed_input: u64, added_output: u64) -> StateTrans<Self> {
        self.input_amount -= removed_input;
        self.output_amount += added_output;
        if self.output_amount >= self.termination_threshold || self.input_amount == 0 {
            StateTrans::EOL
        } else {
            StateTrans::Active(self)
        }
    }
}

impl Fragment for SpotOrder {
    fn side(&self) -> SideM {
        side_of(self.input_asset, self.output_asset)
    }

    fn input(&self) -> u64 {
        self.input_amount
    }

    fn price(&self) -> AbsolutePrice {
        AbsolutePrice::from_price(self.side(), self.base_price)
    }

    fn fee(&self) -> FeePerOutput {
        self.fee_per_output.value()
    }

    fn cost_hint(&self) -> ExecutionCost {
        1 // todo
    }

    fn time_bounds(&self) -> TimeBounds<u64> {
        TimeBounds::None
    }
}

impl Stable for SpotOrder {
    type StableId = PolicyId;
    fn stable_id(&self) -> Self::StableId {
        self.beacon
    }
}

impl Tradable for SpotOrder {
    type PairId = PairId;

    fn pair_id(&self) -> Self::PairId {
        PairId::canonical(self.input_asset, self.output_asset)
    }
}

struct ConfigNativeToToken {
    pub beacon: PolicyId,
    pub output: AssetClass,
    pub termination_threshold: u64,
    pub base_price: RelativePrice,
    pub fee_per_output: FeePerOutput,
    pub redeemer_pkh: Ed25519KeyHash,
    pub redeemer_stake_pkh: Option<Ed25519KeyHash>,
}

impl TryFromPData for ConfigNativeToToken {
    fn try_from_pd(data: PlutusData) -> Option<Self> {
        let mut cpd = data.into_constr_pd()?;
        let stake_pkh: Option<Ed25519KeyHash> = cpd
            .take_field(6)
            .and_then(|pd| pd.into_bytes())
            .and_then(|bytes| <[u8; 28]>::try_from(bytes).ok())
            .map(|bytes| Ed25519KeyHash::from(bytes));

        Some(ConfigNativeToToken {
            beacon: <[u8; 28]>::try_from(cpd.take_field(0)?.into_bytes()?)
                .map(PolicyId::from)
                .ok()?,
            output: AssetClass::try_from_pd(cpd.take_field(1)?)?,
            base_price: RelativePrice::try_from_pd(cpd.take_field(2)?)?,
            fee_per_output: FeePerOutput::try_from_pd(cpd.take_field(3)?)?,
            termination_threshold: cpd.take_field(4)?.into_u64()?,
            redeemer_pkh: Ed25519KeyHash::from(<[u8; 28]>::try_from(cpd.take_field(5)?.into_bytes()?).ok()?),
            redeemer_stake_pkh: stake_pkh,
        })
    }
}

impl TryFromLedger<BabbageTransactionOutput, OutputRef> for SpotOrder {
    fn try_from_ledger(repr: &BabbageTransactionOutput, ctx: OutputRef) -> Option<Self> {
        let value = repr.value().clone();
        let conf = ConfigNativeToToken::try_from_pd(repr.datum()?.into_pd()?)?;
        Some(SpotOrder {
            beacon: conf.beacon,
            input_asset: AssetClass::Native,
            input_amount: value.amount_of(AssetClass::Native)?,
            output_asset: conf.output,
            output_amount: value.amount_of(conf.output)?,
            termination_threshold: conf.termination_threshold,
            base_price: conf.base_price,
            fee_per_output: ExecutorFeePerToken::new(conf.fee_per_output, AssetClass::Native),
            redeemer_pkh: conf.redeemer_pkh,
            redeemer_stake_pkh: conf.redeemer_stake_pkh,
        })
    }
}
