use cbor_event::Sz;
use cml_chain::{
    assets::AssetName,
    plutus::{ConstrPlutusData, PlutusData},
    transaction::TransactionOutput,
    utils::BigInteger,
    PolicyId,
};
use cml_crypto::{RawBytesEncoding, ScriptHash};
use serde::{Deserialize, Serialize};
use spectrum_cardano_lib::{
    plutus_data::{ConstrPlutusDataExtension, DatumExtension, IntoPlutusData, PlutusDataExtension},
    transaction::TransactionOutputExtension,
    types::TryFromPData,
    OutputRef,
};
use spectrum_offchain::{
    data::{Has, HasIdentifier, Identifier},
    ledger::TryFromLedger,
};
use spectrum_offchain_cardano::{
    deployment::{test_address, DeployedScriptInfo},
    parametrized_validators::apply_params_validator,
};
use uplc_pallas_codec::utils::PlutusBytes;

use crate::{
    constants, deployment::ProtocolValidator, entities::Snapshot, protocol_config::FarmFactoryAuthPolicy,
};

pub type FarmFactorySnapshot = Snapshot<FarmFactory, OutputRef>;

#[derive(
    Copy, Clone, PartialEq, Eq, Ord, PartialOrd, Hash, Serialize, Deserialize, Debug, derive_more::Display,
)]
pub struct FarmFactoryId;

impl Identifier for FarmFactoryId {
    type For = FarmFactorySnapshot;
}

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct FarmFactory {
    last_farm_id: i64,
    farm_seed_data: Vec<u8>,
}

impl HasIdentifier for FarmFactorySnapshot {
    type Id = FarmFactoryId;

    fn identifier(&self) -> Self::Id {
        FarmFactoryId
    }
}

impl<C> TryFromLedger<TransactionOutput, C> for FarmFactorySnapshot
where
    C: Has<OutputRef>
        + Has<FarmFactoryAuthPolicy>
        + Has<DeployedScriptInfo<{ ProtocolValidator::FarmFactory as u8 }>>,
{
    fn try_from_ledger(repr: &TransactionOutput, ctx: &C) -> Option<Self> {
        if test_address(repr.address(), ctx) {
            let output_ref = ctx.select::<OutputRef>();
            let datum = repr.datum()?;
            let FarmFactoryDatum {
                last_farm_id,
                farm_seed_data,
            } = datum.into_pd().and_then(FarmFactoryDatum::try_from_pd)?;

            let auth_token_policy_id = ctx.select::<FarmFactoryAuthPolicy>().0;

            let value = repr.value();
            let auth_token_name =
                AssetName::new(constants::DEFAULT_AUTH_TOKEN_NAME.to_be_bytes().to_vec()).unwrap();
            if let Some(quantity) = value.multiasset.get(&auth_token_policy_id, &auth_token_name) {
                if quantity == 1 {
                    let farm_factory = FarmFactory {
                        last_farm_id,
                        farm_seed_data,
                    };

                    return Some(Snapshot::new(farm_factory, output_ref));
                }
            }
        }
        None
    }
}

/// Farm-factory redeemer
pub enum FarmFactoryAction {
    /// Create a new farm.
    CreateFarm,
    /// Leak control over `farm_seed_data` to GovProxy.
    ExecuteProposal,
}

impl IntoPlutusData for FarmFactoryAction {
    fn into_pd(self) -> PlutusData {
        match self {
            FarmFactoryAction::CreateFarm => PlutusData::ConstrPlutusData(ConstrPlutusData::new(0, vec![])),
            FarmFactoryAction::ExecuteProposal => {
                PlutusData::ConstrPlutusData(ConstrPlutusData::new(1, vec![]))
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FarmFactoryDatum {
    pub last_farm_id: i64,
    pub farm_seed_data: Vec<u8>,
}

impl IntoPlutusData for FarmFactoryDatum {
    fn into_pd(self) -> PlutusData {
        let last_farm_id = BigInteger::from(self.last_farm_id);
        let seed_bytes = PlutusData::new_bytes(self.farm_seed_data);
        let cpd = ConstrPlutusData::new(0, vec![PlutusData::new_integer(last_farm_id), seed_bytes]);
        PlutusData::ConstrPlutusData(cpd)
    }
}

impl TryFromPData for FarmFactoryDatum {
    fn try_from_pd(data: PlutusData) -> Option<Self> {
        let mut cpd = data.into_constr_pd()?;
        let last_farm_id = cpd.take_field(0)?.into_i128().unwrap() as i64;
        let farm_seed_data = cpd.take_field(1)?.into_bytes().unwrap();
        Some(Self {
            last_farm_id,
            farm_seed_data,
        })
    }
}

pub fn unsafe_update_farm_factory_datum(data: &mut PlutusData, last_farm_id: i64) {
    let cpd = data.get_constr_pd_mut().unwrap();
    cpd.set_field(0, PlutusData::Integer(BigInteger::from(last_farm_id)));
}

pub fn compute_farm_factory_script_hash(
    script: &str,
    farm_auth_policy: PolicyId,
    gov_witness_script_hash: PolicyId,
) -> ScriptHash {
    let params_pd = uplc::PlutusData::Array(vec![
        uplc::PlutusData::BoundedBytes(PlutusBytes::from(farm_auth_policy.to_raw_bytes().to_vec())),
        uplc::PlutusData::BoundedBytes(PlutusBytes::from(gov_witness_script_hash.to_raw_bytes().to_vec())),
    ]);
    apply_params_validator(params_pd, script)
}

#[cfg(test)]
mod tests {
    use cml_chain::{plutus::PlutusData, Deserialize, Serialize};
    use spectrum_cardano_lib::{plutus_data::IntoPlutusData, types::TryFromPData};

    use super::FarmFactoryDatum;

    #[test]
    fn test_datum_roundtrip() {
        // CBOR encoding of `FarmFactoryDatum` generated by javascript deployment testing
        let last_farm_id = 10_007_199_254_740_992;
        let farm_seed_data =
            hex::decode("581e581c7bf3980a45756eabfb799fd1998f633176f6d2a2e34de887ddb4e8db").unwrap();
        let datum = FarmFactoryDatum {
            last_farm_id,
            farm_seed_data,
        };
        let datum_cbor_bytes = datum.clone().into_pd().to_cbor_bytes();

        // assert_eq!(datum_cbor_bytes, expected_datum_cbor_bytes);
        let pd = PlutusData::from_cbor_bytes(&datum_cbor_bytes).unwrap();
        let generated_datum = FarmFactoryDatum::try_from_pd(pd).unwrap();
        assert_eq!(datum, generated_datum);
    }
}
