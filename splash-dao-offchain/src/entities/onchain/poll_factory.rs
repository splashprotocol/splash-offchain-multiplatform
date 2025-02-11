use cml_chain::plutus::{ConstrPlutusData, PlutusData, PlutusV2Script};

use cml_chain::transaction::TransactionOutput;
use cml_chain::utils::BigInteger;
use cml_chain::PolicyId;
use cml_crypto::{RawBytesEncoding, ScriptHash};
use serde::{Deserialize, Serialize};
use spectrum_cardano_lib::plutus_data::{
    ConstrPlutusDataExtension, DatumExtension, IntoPlutusData, PlutusDataExtension,
};
use spectrum_cardano_lib::types::TryFromPData;
use spectrum_cardano_lib::{AssetName, TaggedAmount};
use spectrum_offchain::domain::{Has, Stable};
use spectrum_offchain::ledger::TryFromLedger;
use spectrum_offchain_cardano::deployment::{test_address, DeployedScriptInfo};
use spectrum_offchain_cardano::parametrized_validators::apply_params_validator_plutus_v2;
use uplc_pallas_primitives::{BoundedBytes, MaybeIndefArray};

use crate::assets::Splash;
use crate::deployment::{DaoScriptData, ProtocolValidator};
use crate::entities::onchain::smart_farm::FarmId;
use crate::entities::onchain::weighting_poll::WeightingPoll;
use crate::entities::Snapshot;
use crate::routines::inflation::TimedOutputRef;
use crate::time::ProtocolEpoch;

pub type PollFactorySnapshot = Snapshot<PollFactory, TimedOutputRef>;

#[derive(
    Copy, Clone, PartialEq, Eq, Ord, PartialOrd, Hash, Serialize, Deserialize, Debug, derive_more::Display,
)]
pub struct PollFactoryId(pub ProtocolEpoch);

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct PollFactory {
    pub last_poll_epoch: Option<ProtocolEpoch>,
    pub active_farms: Vec<FarmId>,
    pub stable_id: ScriptHash,
}

impl PollFactory {
    pub fn next_epoch(&self) -> ProtocolEpoch {
        self.last_poll_epoch.map(|last_epoch| last_epoch + 1).unwrap_or(0)
    }

    pub fn next_weighting_poll(
        mut self,
        emission_rate: TaggedAmount<Splash>,
    ) -> (PollFactory, WeightingPoll) {
        let next_epoch = self.next_epoch();
        let next_poll = WeightingPoll {
            epoch: next_epoch,
            distribution: self.active_farms.iter().map(|farm| (*farm, 0u64)).collect(),
            emission_rate,
            weighting_power: None,
            eliminated: false,
        };
        self.last_poll_epoch = Some(next_epoch);
        (self, next_poll)
    }
}

impl<C> TryFromLedger<TransactionOutput, C> for PollFactorySnapshot
where
    C: Has<TimedOutputRef> + Has<DeployedScriptInfo<{ ProtocolValidator::WpFactory as u8 }>>,
{
    fn try_from_ledger(repr: &TransactionOutput, ctx: &C) -> Option<Self> {
        if test_address(repr.address(), ctx) {
            let version = ctx.select::<TimedOutputRef>();
            let datum = repr.datum()?;
            let PollFactoryConfig {
                last_poll_epoch,
                active_farms,
            } = datum
                .into_pd()
                .map(|pd| PollFactoryConfig::try_from_pd(pd).unwrap())?;

            let last_poll_epoch = if last_poll_epoch < 0 {
                None
            } else {
                Some(last_poll_epoch as u32)
            };

            let poll_factory = PollFactory {
                last_poll_epoch,
                active_farms,
                stable_id: ctx
                    .select::<DeployedScriptInfo<{ ProtocolValidator::WpFactory as u8 }>>()
                    .script_hash,
            };

            return Some(Snapshot::new(poll_factory, version));
        }
        None
    }
}

impl Stable for PollFactory {
    type StableId = PollFactoryId;
    fn stable_id(&self) -> Self::StableId {
        let epoch = self.last_poll_epoch.map(|e| e + 1).unwrap_or(0);
        PollFactoryId(epoch)
    }
    fn is_quasi_permanent(&self) -> bool {
        true
    }
}

pub fn unsafe_update_factory_state(data: &mut PlutusData, last_poll_epoch: ProtocolEpoch) {
    let cpd = data.get_constr_pd_mut().unwrap();
    cpd.set_field(0, PlutusData::new_integer(last_poll_epoch.into()))
}

pub enum PollFactoryAction {
    CreatePoll,
    ExecuteProposal,
}

impl IntoPlutusData for PollFactoryAction {
    fn into_pd(self) -> PlutusData {
        match self {
            PollFactoryAction::CreatePoll => PlutusData::ConstrPlutusData(ConstrPlutusData::new(0, vec![])),
            PollFactoryAction::ExecuteProposal => {
                PlutusData::ConstrPlutusData(ConstrPlutusData::new(1, vec![]))
            }
        }
    }
}

pub struct FactoryRedeemer {
    pub successor_ix: usize,
    pub action: PollFactoryAction,
}

impl IntoPlutusData for FactoryRedeemer {
    fn into_pd(self) -> PlutusData {
        let cpd = ConstrPlutusData::new(
            0,
            vec![(self.successor_ix as u64).into_pd(), self.action.into_pd()],
        );
        //cpd.set_field(
        //    PF_REDEEMER_MAPPING.action,
        //    PlutusData::new_list(vec![self.action.into_pd()]),
        //);
        PlutusData::ConstrPlutusData(cpd)
    }
}

pub fn compute_wp_factory_validator(
    wp_auth_policy: PolicyId,
    gov_witness_script_hash: ScriptHash,
) -> PlutusV2Script {
    let params_pd = uplc::PlutusData::Array(MaybeIndefArray::Indef(vec![
        uplc::PlutusData::BoundedBytes(BoundedBytes::from(wp_auth_policy.to_raw_bytes().to_vec())),
        uplc::PlutusData::BoundedBytes(BoundedBytes::from(
            gov_witness_script_hash.to_raw_bytes().to_vec(),
        )),
    ]));
    apply_params_validator_plutus_v2(params_pd, &DaoScriptData::global().wp_factory.script_bytes)
}

pub struct PollFactoryConfig {
    /// Epoch of the last WP.
    pub last_poll_epoch: i32,
    /// Active farms.
    pub active_farms: Vec<FarmId>,
}

impl IntoPlutusData for PollFactoryConfig {
    fn into_pd(self) -> PlutusData {
        let last_poll_epoch = PlutusData::new_integer(BigInteger::from(self.last_poll_epoch));
        let active_farms: Vec<_> = self
            .active_farms
            .into_iter()
            .map(|FarmId(id)| {
                let bytes = cml_chain::assets::AssetName::from(id).to_raw_bytes().to_vec();
                PlutusData::new_bytes(bytes)
            })
            .collect();

        PlutusData::ConstrPlutusData(ConstrPlutusData::new(
            0,
            vec![last_poll_epoch, PlutusData::new_list(active_farms)],
        ))
    }
}

impl TryFromPData for PollFactoryConfig {
    fn try_from_pd(data: cml_chain::plutus::PlutusData) -> Option<Self> {
        let mut cpd = data.into_constr_pd()?;
        let last_poll_epoch = cpd.take_field(0)?.into_i128().unwrap() as i32;
        let active_farms: Vec<_> = cpd
            .take_field(1)?
            .into_vec()
            .unwrap()
            .into_iter()
            .map(|pd| FarmId(AssetName::try_from(pd.into_bytes().unwrap()).unwrap()))
            .collect();
        Some(Self {
            last_poll_epoch,
            active_farms,
        })
    }
}

#[cfg(test)]
mod tests {
    use cbor_event::de::Deserializer;
    use cml_chain::{plutus::PlutusData, Deserialize};
    use spectrum_cardano_lib::types::TryFromPData;
    use std::io::Cursor;

    use super::PollFactoryConfig;

    #[test]
    fn test_poll_factory_datum_deserialization() {
        // Hex-encoded datum from preprod deployment TX
        let bytes = hex::decode("d8799f009f456661726d30426631ffff").unwrap();
        let mut raw = Deserializer::from(Cursor::new(bytes));

        let data = PlutusData::deserialize(&mut raw).unwrap();
        assert!(PollFactoryConfig::try_from_pd(data).is_some());
    }
}
