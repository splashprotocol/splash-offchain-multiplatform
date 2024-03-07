use cml_chain::plutus::PlutusData;

use spectrum_cardano_lib::plutus_data::IntoPlutusData;
use spectrum_cardano_lib::{TaggedAmount, Token};
use spectrum_offchain::data::{EntitySnapshot, Identifier, Stable};

use crate::assets::Splash;
use crate::time::{epoch_end, NetworkTime, ProtocolEpoch};
use crate::{constants, GenesisEpochStartTime};

#[derive(Copy, Clone, PartialEq, Eq, Ord, PartialOrd)]
pub struct InflationBoxId(Token);

impl Identifier for InflationBoxId {
    type For = InflationBox;
}

#[derive(Copy, Clone, Debug)]
pub struct InflationBox {
    pub last_processed_epoch: ProtocolEpoch,
    pub splash_reserves: TaggedAmount<Splash>,
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
        let exp = [0..reduction_period - 2];
        // We calculate numerator/denominator separately to avoid error accumulation.
        let num = exp.iter().fold(constants::RATE_AFTER_FIRST_REDUCTION, |acc, _| {
            acc * constants::TAIL_REDUCTION_RATE_NUM
        });
        let denom = exp
            .iter()
            .fold(1, |acc, _| acc * constants::TAIL_REDUCTION_RATE_DEN);
        num / denom
    })
}

impl Stable for InflationBox {
    type StableId = u64;
    fn stable_id(&self) -> Self::StableId {
        todo!()
    }
}

impl EntitySnapshot for InflationBox {
    type Version = u64;
    fn version(&self) -> Self::Version {
        todo!()
    }
}

pub fn unsafe_update_ibox_state(data: &mut PlutusData, last_processed_epoch: ProtocolEpoch) {
    *data = last_processed_epoch.into_pd();
}
