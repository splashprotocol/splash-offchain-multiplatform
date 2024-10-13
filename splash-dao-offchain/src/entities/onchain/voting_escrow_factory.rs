use std::ops::Deref;

use cml_chain::{
    plutus::{ConstrPlutusData, PlutusData, PlutusMap, PlutusV2Script},
    transaction::TransactionOutput,
    utils::BigInteger,
    PolicyId, Value,
};
use cml_core::serialization::RawBytesEncoding;
use cml_crypto::{blake2b256, ScriptHash};
use num_rational::Ratio;
use serde::{Deserialize, Serialize};
use spectrum_cardano_lib::{
    plutus_data::{ConstrPlutusDataExtension, DatumExtension, IntoPlutusData, PlutusDataExtension},
    transaction::TransactionOutputExtension,
    types::TryFromPData,
    value::ValueExtension,
    AssetClass, AssetName, OutputRef, Token,
};
use spectrum_offchain::{
    data::{Has, Identifier},
    ledger::TryFromLedger,
};
use spectrum_offchain_cardano::{
    deployment::{test_address, DeployedScriptInfo},
    parametrized_validators::apply_params_validator,
};
use uplc_pallas_codec::utils::PlutusBytes;

use crate::{
    constants::{DEFAULT_AUTH_TOKEN_NAME, GT_NAME, VE_FACTORY_SCRIPT},
    deployment::ProtocolValidator,
    entities::Snapshot,
    protocol_config::{GTAuthName, GTAuthPolicy, VEFactoryAuthName, VEFactoryAuthPolicy},
};

pub type VEFactorySnapshot = Snapshot<VEFactory, OutputRef>;

#[derive(
    Copy, Clone, PartialEq, Eq, Ord, PartialOrd, Hash, Serialize, Deserialize, Debug, derive_more::Display,
)]
pub struct VEFactoryId;

impl Identifier for VEFactoryId {
    type For = VEFactorySnapshot;
}

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct VEFactory {
    pub accepted_assets: Vec<(Token, Ratio<u128>)>,
    pub legacy_accepted_assets: Vec<(Token, Ratio<u128>)>,
    pub accepted_assets_inventory: Vec<(Token, u64)>,
    pub legacy_assets_inventory: Vec<(Token, u64)>,
    pub gt_tokens_available: u64,
}

impl<C> TryFromLedger<TransactionOutput, C> for VEFactorySnapshot
where
    C: Has<OutputRef>
        + Has<VEFactoryAuthPolicy>
        + Has<GTAuthPolicy>
        + Has<DeployedScriptInfo<{ ProtocolValidator::VeFactory as u8 }>>,
{
    fn try_from_ledger(repr: &TransactionOutput, ctx: &C) -> Option<Self> {
        if test_address(repr.address(), ctx) {
            let datum = repr.datum()?;
            let VEFactoryDatum {
                accepted_assets,
                legacy_accepted_assets,
            } = datum.into_pd().and_then(VEFactoryDatum::try_from_pd)?;

            let gt_policy_id = ctx.select::<GTAuthPolicy>().0;
            let gt_asset_name = cml_chain::assets::AssetName::new(GT_NAME.to_be_bytes().to_vec()).unwrap();

            let auth_token_policy_id = ctx.select::<VEFactoryAuthPolicy>().0;
            let auth_token_name =
                spectrum_cardano_lib::AssetName::try_from(DEFAULT_AUTH_TOKEN_NAME.to_be_bytes().to_vec())
                    .unwrap();
            let expected_auth_token = Token(auth_token_policy_id, auth_token_name);
            let mut accepted_assets_inventory = vec![];
            let mut legacy_assets_inventory = vec![];
            let mut gt_tokens_available = None;
            let mut auth_token_present = false;
            let value = repr.value();
            for (policy_id, by_names) in value.multiasset.iter() {
                if gt_tokens_available.is_none() && *policy_id == gt_policy_id && by_names.len() == 1 {
                    gt_tokens_available = by_names.deref().get(&gt_asset_name).copied();
                } else {
                    for (token_name, qty) in by_names.iter() {
                        let token_name = spectrum_cardano_lib::AssetName::from(token_name.clone());
                        let token = Token(*policy_id, token_name);
                        if token == expected_auth_token {
                            auth_token_present = true;
                        } else if is_token_accepted(token, &accepted_assets) {
                            accepted_assets_inventory.push((token, *qty));
                        } else if is_token_accepted(token, &legacy_accepted_assets) {
                            legacy_assets_inventory.push((token, *qty));
                        } else {
                            // Token isn't accepted by ve_factory.
                            return None;
                        }
                    }
                }
            }
            if !auth_token_present {
                return None;
            }
            let gt_tokens_available = gt_tokens_available?;
            let ve_factory = VEFactory {
                accepted_assets,
                legacy_accepted_assets,
                accepted_assets_inventory,
                legacy_assets_inventory,
                gt_tokens_available,
            };
            let output_ref = ctx.select::<OutputRef>();
            return Some(Snapshot::new(ve_factory, output_ref));
        }
        None
    }
}

pub fn compute_ve_factory_validator(
    ve_factory_auth_policy: PolicyId,
    ve_identifier_policy: PolicyId,
    ve_composition_policy: PolicyId,
    gt_policy: PolicyId,
    voting_escrow_scripthash: ScriptHash,
    gov_proxy_scripthash: ScriptHash,
) -> PlutusV2Script {
    let params_pd = uplc::PlutusData::Array(vec![
        uplc::PlutusData::BoundedBytes(PlutusBytes::from(ve_factory_auth_policy.to_raw_bytes().to_vec())),
        uplc::PlutusData::BoundedBytes(PlutusBytes::from(ve_identifier_policy.to_raw_bytes().to_vec())),
        uplc::PlutusData::BoundedBytes(PlutusBytes::from(ve_composition_policy.to_raw_bytes().to_vec())),
        uplc::PlutusData::BoundedBytes(PlutusBytes::from(gt_policy.to_raw_bytes().to_vec())),
        uplc::PlutusData::BoundedBytes(PlutusBytes::from(
            voting_escrow_scripthash.to_raw_bytes().to_vec(),
        )),
        uplc::PlutusData::BoundedBytes(PlutusBytes::from(gov_proxy_scripthash.to_raw_bytes().to_vec())),
    ]);
    apply_params_validator(params_pd, VE_FACTORY_SCRIPT)
}

fn is_token_accepted(token: Token, accepted_assets: &[(Token, Ratio<u128>)]) -> bool {
    accepted_assets
        .iter()
        .any(|&(acceptable_token, _)| token == acceptable_token)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VEFactoryDatum {
    pub accepted_assets: Vec<(Token, Ratio<u128>)>,
    pub legacy_accepted_assets: Vec<(Token, Ratio<u128>)>,
}

impl IntoPlutusData for VEFactoryDatum {
    fn into_pd(self) -> cml_chain::plutus::PlutusData {
        let mut accepted_assets_map = PlutusMap::new();
        let mut legacy_accepted_assets_map = PlutusMap::new();
        for kv in self.accepted_assets {
            let (token, ratio) = accepted_asset_to_pd(kv);
            accepted_assets_map.set(token, ratio);
        }

        for kv in self.legacy_accepted_assets {
            let (token, ratio) = accepted_asset_to_pd(kv);
            legacy_accepted_assets_map.set(token, ratio);
        }
        let cpd = ConstrPlutusData::new(
            0,
            vec![
                PlutusData::new_map(accepted_assets_map),
                PlutusData::new_map(legacy_accepted_assets_map),
            ],
        );
        PlutusData::ConstrPlutusData(cpd)
    }
}

impl TryFromPData for VEFactoryDatum {
    fn try_from_pd(data: PlutusData) -> Option<Self> {
        let mut cpd = data.into_constr_pd()?;
        let accepted_assets_map = cpd.take_field(0)?.into_pd_map().unwrap();
        let mut accepted_assets = vec![];
        for pair in accepted_assets_map {
            let accepted_asset = pd_to_accepted_asset(pair)?;
            accepted_assets.push(accepted_asset);
        }

        let legacy_accepted_assets_map = cpd.take_field(1)?.into_pd_map().unwrap();
        let mut legacy_accepted_assets = vec![];
        for pair in legacy_accepted_assets_map {
            let accepted_asset = pd_to_accepted_asset(pair)?;
            legacy_accepted_assets.push(accepted_asset);
        }
        Some(Self {
            accepted_assets,
            legacy_accepted_assets,
        })
    }
}

#[derive(Serialize, Deserialize, Clone)]
pub struct AcceptedAsset {
    pub asset_name_utf8: String,
    pub policy_id: ScriptHash,
    pub exchange_rate: Ratio<u128>,
}

impl From<Vec<AcceptedAsset>> for VEFactoryDatum {
    fn from(value: Vec<AcceptedAsset>) -> Self {
        let accepted_assets = value
            .into_iter()
            .map(
                |AcceptedAsset {
                     asset_name_utf8,
                     policy_id,
                     exchange_rate,
                 }| {
                    let asset_name = AssetName::utf8_unsafe(asset_name_utf8);
                    (Token(policy_id, asset_name), exchange_rate)
                },
            )
            .collect();

        Self {
            accepted_assets,
            legacy_accepted_assets: vec![],
        }
    }
}

pub enum FactoryAction {
    /// Deposit LQ* for a desirable period and get voting power locked in VE in exchange.
    Deposit,
    /// Add more LQ* into an existing VE.
    ExtendPosition {
        /// Index of the VE input.
        ve_in_ix: u64,
    },
    /// Return voting power locked in VE and get back LQ* if lock has expired.
    RedeemFromVE {
        /// Index of the VE input.
        ve_in_ix: u64,
    },
    /// Leak control over factory configuration to goveranance.
    ExecuteProposal,
}

impl IntoPlutusData for FactoryAction {
    fn into_pd(self) -> PlutusData {
        PlutusData::ConstrPlutusData(match self {
            FactoryAction::Deposit => ConstrPlutusData::new(0, vec![]),
            FactoryAction::ExtendPosition { ve_in_ix } => ConstrPlutusData::new(1, vec![ve_in_ix.into_pd()]),
            FactoryAction::RedeemFromVE { ve_in_ix } => ConstrPlutusData::new(2, vec![ve_in_ix.into_pd()]),
            FactoryAction::ExecuteProposal => ConstrPlutusData::new(3, vec![]),
        })
    }
}

fn accepted_asset_to_pd((Token(token_id, name), ratio): (Token, Ratio<u128>)) -> (PlutusData, PlutusData) {
    let token_cpd = ConstrPlutusData::new(
        0,
        vec![
            PlutusData::new_bytes(token_id.to_raw_bytes().to_vec()),
            PlutusData::new_bytes(cml_chain::assets::AssetName::from(name).to_raw_bytes().to_vec()),
        ],
    );
    let ratio_cpd = ConstrPlutusData::new(
        0,
        vec![
            PlutusData::new_integer(BigInteger::from(*ratio.numer())),
            PlutusData::new_integer(BigInteger::from(*ratio.denom())),
        ],
    );
    (
        PlutusData::ConstrPlutusData(token_cpd),
        PlutusData::ConstrPlutusData(ratio_cpd),
    )
}

fn pd_to_accepted_asset((key_pd, value_pd): (PlutusData, PlutusData)) -> Option<(Token, Ratio<u128>)> {
    let mut token_cpd = key_pd.into_constr_pd()?;
    let token_id_bytes: [u8; 28] = token_cpd.take_field(0)?.into_bytes()?.try_into().ok()?;
    let token_id = ScriptHash::from(token_id_bytes);
    let asset_name_bytes = token_cpd.take_field(1)?.into_bytes()?;
    let cml_asset_name = cml_chain::assets::AssetName::from_raw_bytes(&asset_name_bytes).ok()?;
    let asset_name = spectrum_cardano_lib::AssetName::from(cml_asset_name);
    let token = Token(token_id, asset_name);

    let mut ratio_cpd = value_pd.into_constr_pd()?;
    let numer = ratio_cpd.take_field(0)?.into_u128()?;
    let denom = ratio_cpd.take_field(1)?.into_u128()?;
    let ratio = Ratio::new_raw(numer, denom);
    Some((token, ratio))
}

/// Compute total ve_composition based on changes in balances of accepted assets and expected minted
/// value of individual ve_composition tokens.
pub fn exchange_outputs(
    self_value: &Value,
    succ_value: &Value,
    accepted_deposits: Vec<(Token, Ratio<u128>)>,
    ve_composition_policy: PolicyId,
    expect_redeem: bool,
) -> (u64, Value) {
    let mut ve_composition_qty = 0;
    let mut mint_value = Value::zero();
    for (token, ratio) in accepted_deposits {
        let asset_class = AssetClass::from(token);
        let pred = self_value.amount_of(asset_class).unwrap_or(0) as i128;
        let succ = succ_value.amount_of(asset_class).unwrap_or(0) as i128;
        let delta = succ - pred;
        assert!(expect_redeem && delta < 0 || delta > 0);
        if delta != 0 {
            let mut bytes = token.0.to_raw_bytes().to_vec();
            bytes.extend(cml_chain::assets::AssetName::from(token.1).to_raw_bytes());
            let ve_composition_tn = AssetName::try_from(blake2b256(&bytes).to_vec()).unwrap();
            let ve_comp_token = AssetClass::from(Token(ve_composition_policy, ve_composition_tn));
            mint_value.add_unsafe(ve_comp_token, delta as u64);
        }
        ve_composition_qty += (delta * (*ratio.numer() as i128) / (*ratio.denom() as i128)) as u64;
    }

    (ve_composition_qty, mint_value)
}

#[cfg(test)]
mod tests {
    use cml_crypto::ScriptHash;
    use num_rational::Ratio;
    use rand::Rng;
    use spectrum_cardano_lib::{plutus_data::IntoPlutusData, types::TryFromPData, Token};

    use super::VEFactoryDatum;

    #[test]
    fn test_datum_roundtrip() {
        let accepted_assets = vec![
            (gen_token(), Ratio::new_raw(100, 2000)),
            (gen_token(), Ratio::new_raw(1000000, 2000000000000)),
            (gen_token(), Ratio::new_raw(1002345, 30000000000)),
            (gen_token(), Ratio::new_raw(100234324, 2000)),
        ];
        let legacy_accepted_assets = vec![(gen_token(), Ratio::new_raw(100, 2000))];
        let datum = VEFactoryDatum {
            accepted_assets,
            legacy_accepted_assets,
        };

        let pd = datum.clone().into_pd();
        assert_eq!(datum, VEFactoryDatum::try_from_pd(pd).unwrap());
    }

    fn gen_token() -> Token {
        let mut rng = rand::thread_rng();
        let token_id_bytes: [u8; 28] = rng.gen();
        let token_id = ScriptHash::from(token_id_bytes);
        let asset_name = spectrum_cardano_lib::AssetName::try_from_hex("a4").unwrap();
        Token(token_id, asset_name)
    }
}
