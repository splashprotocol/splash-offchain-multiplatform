use cml_chain::assets::AssetName;
use cml_chain::plutus::{ExUnits, PlutusData, PlutusV2Script};

use cml_chain::transaction::TransactionOutput;
use cml_chain::PolicyId;
use cml_crypto::{RawBytesEncoding, ScriptHash};
use log::trace;
use primitive_types::U512;
use serde::{Deserialize, Serialize};
use spectrum_cardano_lib::plutus_data::{DatumExtension, IntoPlutusData, PlutusDataExtension};
use spectrum_cardano_lib::transaction::TransactionOutputExtension;
use spectrum_cardano_lib::{OutputRef, TaggedAmount, Token};
use spectrum_offchain::domain::{EntitySnapshot, Has, Stable};
use spectrum_offchain::ledger::TryFromLedger;
use spectrum_offchain_cardano::deployment::{test_address, DeployedScriptInfo};
use spectrum_offchain_cardano::parametrized_validators::apply_params_validator;
use uplc_pallas_codec::utils::{Int, PlutusBytes};

use crate::assets::Splash;
use crate::constants::time::EPOCH_BOUNDARY_SHIFT;
use crate::constants::{script_bytes::INFLATION_SCRIPT, SPLASH_NAME};
use crate::deployment::ProtocolValidator;
use crate::entities::Snapshot;
use crate::protocol_config::SplashPolicy;
use crate::routines::inflation::{Slot, TimedOutputRef};
use crate::time::{epoch_end, NetworkTime, ProtocolEpoch};
use crate::{constants, GenesisEpochStartTime};

pub type InflationBoxSnapshot = Snapshot<InflationBox, TimedOutputRef>;

#[derive(
    Copy, Clone, Debug, PartialEq, Eq, Ord, PartialOrd, Serialize, Deserialize, Hash, derive_more::Display,
)]
pub struct InflationBoxId(pub ProtocolEpoch);

#[derive(Copy, Clone, Debug, Serialize, Deserialize)]
pub struct InflationBox {
    pub last_processed_epoch: Option<ProtocolEpoch>,
    pub splash_reserves: TaggedAmount<Splash>,
    pub script_hash: ScriptHash,
}

impl InflationBox {
    pub fn active_epoch(&self, genesis: GenesisEpochStartTime, now: NetworkTime) -> ProtocolEpoch {
        if let Some(last_processed_epoch) = self.last_processed_epoch {
            if epoch_end(genesis, last_processed_epoch) > now - EPOCH_BOUNDARY_SHIFT {
                last_processed_epoch
            } else {
                last_processed_epoch + 1
            }
        } else {
            0
        }
    }

    pub fn release_next_tranche(mut self) -> (InflationBox, TaggedAmount<Splash>) {
        let next_epoch = if self.last_processed_epoch.is_none() {
            0
        } else {
            self.last_processed_epoch.unwrap() + 1
        };
        let rate = emission_rate(next_epoch);
        self.last_processed_epoch = Some(next_epoch);
        self.splash_reserves -= rate;
        (self, rate)
    }
}

/// Calculate emission rate based on given epoch.
pub fn emission_rate(epoch: ProtocolEpoch) -> TaggedAmount<Splash> {
    let reduction_period = epoch / constants::EMISSION_REDUCTION_PERIOD_LEN;
    TaggedAmount::new(if reduction_period == 0 {
        constants::RATE_INITIAL
    } else if reduction_period == 1 {
        constants::RATE_AFTER_FIRST_REDUCTION
    } else {
        let exp = U512::from(reduction_period - 1);
        // We calculate numerator/denominator separately to avoid error accumulation.
        let num = U512::from(constants::RATE_AFTER_FIRST_REDUCTION)
            * U512::from(constants::TAIL_REDUCTION_RATE_NUM).pow(exp);
        let denom = U512::from(constants::TAIL_REDUCTION_RATE_DEN).pow(exp);
        (num / denom).as_u64()
    })
}

impl Stable for InflationBox {
    type StableId = InflationBoxId;
    fn stable_id(&self) -> Self::StableId {
        let epoch = self.last_processed_epoch.map(|e| e + 1).unwrap_or(0);
        InflationBoxId(epoch)
    }
    fn is_quasi_permanent(&self) -> bool {
        true
    }
}

impl<C> TryFromLedger<TransactionOutput, C> for InflationBoxSnapshot
where
    C: Has<SplashPolicy>
        + Has<DeployedScriptInfo<{ ProtocolValidator::Inflation as u8 }>>
        + Has<TimedOutputRef>,
{
    fn try_from_ledger(repr: &TransactionOutput, ctx: &C) -> Option<Self> {
        if test_address(repr.address(), ctx) {
            let value = repr.value().clone();
            let datum = repr.datum()?;
            let epoch = datum.into_pd()?.into_u64()?;
            let last_processed_epoch = if epoch > 0 { Some(epoch as u32 - 1) } else { None };
            let splash_asset_name = AssetName::try_from(SPLASH_NAME).unwrap();
            let splash = value
                .multiasset
                .get(&ctx.select::<SplashPolicy>().0, &splash_asset_name)?;
            let script_hash = repr.script_hash()?;

            let inflation_box = InflationBox {
                last_processed_epoch,
                splash_reserves: TaggedAmount::new(splash),
                script_hash,
            };
            let version = ctx.select::<TimedOutputRef>();

            return Some(Snapshot::new(inflation_box, version));
        }
        None
    }
}

pub fn unsafe_update_ibox_state(data: &mut PlutusData, last_processed_epoch: ProtocolEpoch) {
    *data = PlutusData::new_integer(last_processed_epoch.into());
}

pub const INFLATION_BOX_EX_UNITS: ExUnits = ExUnits {
    mem: 500_000,
    steps: 200_000_000,
    encodings: None,
};

pub fn compute_inflation_box_validator(
    inflation_auth_policy: PolicyId,
    splash_policy: PolicyId,
    wp_auth_policy: PolicyId,
    weighting_power_policy: PolicyId,
    zeroth_epoch_start: u64,
) -> PlutusV2Script {
    let params_pd = uplc::PlutusData::Array(vec![
        uplc::PlutusData::BoundedBytes(PlutusBytes::from(inflation_auth_policy.to_raw_bytes().to_vec())),
        uplc::PlutusData::BoundedBytes(PlutusBytes::from(splash_policy.to_raw_bytes().to_vec())),
        uplc::PlutusData::BoundedBytes(PlutusBytes::from(wp_auth_policy.to_raw_bytes().to_vec())),
        uplc::PlutusData::BoundedBytes(PlutusBytes::from(weighting_power_policy.to_raw_bytes().to_vec())),
        uplc::PlutusData::BigInt(uplc::BigInt::Int(Int::from(zeroth_epoch_start as i64))),
    ]);
    apply_params_validator(params_pd, INFLATION_SCRIPT)
}

#[cfg(test)]
mod tests {
    use crate::entities::onchain::inflation_box::emission_rate;

    #[test]
    fn check_emission() {
        for e in 0..364 {
            println!("epoch {} emission: {:?}", e, emission_rate(e));
        }
    }
}
