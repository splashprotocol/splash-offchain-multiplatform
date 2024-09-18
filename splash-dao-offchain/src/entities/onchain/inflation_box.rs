use cml_chain::assets::AssetName;
use cml_chain::plutus::{ExUnits, PlutusData};

use cml_chain::PolicyId;
use cml_crypto::{RawBytesEncoding, ScriptHash};
use cml_multi_era::babbage::BabbageTransactionOutput;
use serde::{Deserialize, Serialize};
use spectrum_cardano_lib::plutus_data::{DatumExtension, IntoPlutusData, PlutusDataExtension};
use spectrum_cardano_lib::transaction::TransactionOutputExtension;
use spectrum_cardano_lib::{OutputRef, TaggedAmount, Token};
use spectrum_offchain::data::{EntitySnapshot, Has, HasIdentifier, Identifier, Stable};
use spectrum_offchain::ledger::TryFromLedger;
use spectrum_offchain_cardano::deployment::{test_address, DeployedScriptInfo};
use spectrum_offchain_cardano::parametrized_validators::apply_params_validator;
use uplc_pallas_codec::utils::{Int, PlutusBytes};

use crate::assets::Splash;
use crate::constants::SPLASH_NAME;
use crate::deployment::ProtocolValidator;
use crate::entities::Snapshot;
use crate::protocol_config::{SplashAssetName, SplashPolicy};
use crate::time::{epoch_end, NetworkTime, ProtocolEpoch};
use crate::{constants, GenesisEpochStartTime};

pub type InflationBoxSnapshot = Snapshot<InflationBox, OutputRef>;

#[derive(Copy, Clone, PartialEq, Eq, Ord, PartialOrd, Serialize, Deserialize)]
pub struct InflationBoxId;

impl Identifier for InflationBoxId {
    type For = InflationBoxSnapshot;
}

#[derive(Copy, Clone, Debug, Serialize, Deserialize)]
pub struct InflationBox {
    pub last_processed_epoch: ProtocolEpoch,
    pub splash_reserves: TaggedAmount<Splash>,
    pub script_hash: ScriptHash,
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

impl HasIdentifier for InflationBoxSnapshot {
    type Id = InflationBoxId;

    fn identifier(&self) -> Self::Id {
        InflationBoxId
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
    type StableId = ScriptHash;
    fn stable_id(&self) -> Self::StableId {
        self.script_hash
    }
    fn is_quasi_permanent(&self) -> bool {
        true
    }
}

impl<C> TryFromLedger<BabbageTransactionOutput, C> for InflationBoxSnapshot
where
    C: Has<SplashPolicy>
        + Has<SplashAssetName>
        + Has<DeployedScriptInfo<{ ProtocolValidator::Inflation as u8 }>>
        + Has<OutputRef>,
{
    fn try_from_ledger(repr: &BabbageTransactionOutput, ctx: &C) -> Option<Self> {
        if test_address(repr.address(), ctx) {
            println!("INFLATION_BOX ADDR OK!!");
            let value = repr.value().clone();
            println!("aaa");
            let datum = repr.datum()?;
            println!("bbb: datum: {:?}", datum);
            let epoch = datum.into_pd()?.into_u64()?;
            println!("ccc");
            let splash = value.multiasset.get(
                &ctx.select::<SplashPolicy>().0,
                &ctx.select::<SplashAssetName>().0,
            )?;
            println!("ddd");
            let script_hash = repr.script_hash()?;
            println!("eee");

            let inflation_box = InflationBox {
                last_processed_epoch: epoch as u32,
                splash_reserves: TaggedAmount::new(splash),
                script_hash,
            };
            let output_ref = ctx.select::<OutputRef>();

            return Some(Snapshot::new(inflation_box, output_ref));
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

pub fn compute_inflation_box_script_hash(
    script: &str,
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
    apply_params_validator(params_pd, script)
}

#[cfg(test)]
mod tests {
    use crate::entities::onchain::inflation_box::emission_rate;

    #[test]
    fn check_emission() {
        println!("epoch 0 emission: {:?}", emission_rate(0));
    }
}
