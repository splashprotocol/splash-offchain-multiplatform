use cml_chain::plutus::{ExUnits, PlutusData};

use cml_chain::PolicyId;
use cml_crypto::{RawBytesEncoding, ScriptHash};
use spectrum_cardano_lib::plutus_data::IntoPlutusData;
use spectrum_cardano_lib::{TaggedAmount, Token};
use spectrum_offchain::data::{EntitySnapshot, Identifier, Stable};
use spectrum_offchain_cardano::parametrized_validators::apply_params_validator;
use uplc_pallas_codec::utils::{Int, PlutusBytes};

use crate::assets::Splash;
use crate::constants::INFLATION_SCRIPT;
use crate::routines::inflation::InflationBoxSnapshot;
use crate::time::{epoch_end, NetworkTime, ProtocolEpoch};
use crate::{constants, GenesisEpochStartTime};

#[derive(Copy, Clone, PartialEq, Eq, Ord, PartialOrd)]
pub struct InflationBoxId(Token);

impl Identifier for InflationBoxId {
    type For = InflationBoxSnapshot;
}

#[derive(Copy, Clone, Debug)]
pub struct InflationBox {
    pub last_processed_epoch: ProtocolEpoch,
    pub splash_reserves: TaggedAmount<Splash>,
    pub wp_auth_policy: PolicyId,
}

impl InflationBox {
    pub fn active_epoch(&self, genesis: GenesisEpochStartTime, now: NetworkTime) -> ProtocolEpoch {
        if epoch_end(genesis, self.last_processed_epoch) < now {
            self.last_processed_epoch
        } else {
            self.last_processed_epoch + 1
        }
    }

    pub fn release_next_tranche(mut self) -> (InflationBox, TaggedAmount<Splash>) {
        let next_epoch = self.last_processed_epoch + 1;
        let rate = emission_rate(next_epoch);
        self.last_processed_epoch = next_epoch;
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
        let exp = reduction_period - 1;
        // We calculate numerator/denominator separately to avoid error accumulation.
        let num = constants::RATE_AFTER_FIRST_REDUCTION * constants::TAIL_REDUCTION_RATE_NUM.pow(exp);
        let denom = constants::TAIL_REDUCTION_RATE_DEN.pow(exp);
        num / denom
    })
}

impl Stable for InflationBox {
    type StableId = PolicyId;
    fn stable_id(&self) -> Self::StableId {
        self.wp_auth_policy
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

pub fn compute_inflation_box_script_hash(
    splash_policy: PolicyId,
    wp_auth_policy: PolicyId,
    weighting_power_policy: PolicyId,
    zeroth_epoch_start: u64,
) -> ScriptHash {
    let params_pd = uplc::PlutusData::Array(vec![
        uplc::PlutusData::BoundedBytes(PlutusBytes::from(splash_policy.to_raw_bytes().to_vec())),
        uplc::PlutusData::BoundedBytes(PlutusBytes::from(wp_auth_policy.to_raw_bytes().to_vec())),
        uplc::PlutusData::BoundedBytes(PlutusBytes::from(weighting_power_policy.to_raw_bytes().to_vec())),
        uplc::PlutusData::BigInt(uplc::BigInt::Int(Int::from(zeroth_epoch_start as i64))),
    ]);
    apply_params_validator(params_pd, INFLATION_SCRIPT)
}
