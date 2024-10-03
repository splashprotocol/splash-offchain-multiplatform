use cardano_explorer::CardanoNetwork;
use cml_chain::utils::BigInteger;
use cml_crypto::ScriptHash;
use spectrum_offchain_cardano::deployment::{
    DeployedScriptInfo, DeployedValidator, DeployedValidatorRef, Script,
};

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DaoDeployment {
    pub validators: DeployedValidators,
    pub nfts: MintedTokens,
}

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeployedValidators {
    pub inflation: DeployedValidatorRef,
    pub voting_escrow: DeployedValidatorRef,
    pub smart_farm: DeployedValidatorRef,
    pub farm_factory: DeployedValidatorRef,
    pub wp_factory: DeployedValidatorRef,
    pub ve_factory: DeployedValidatorRef,
    pub gov_proxy: DeployedValidatorRef,
    pub perm_manager: DeployedValidatorRef,
    #[serde(rename(deserialize = "mintWPAuthToken"))]
    pub mint_wpauth_token: DeployedValidatorRef,
    //#[serde(rename(deserialize = "mintVEIdentifierToken"))]
    //pub mint_ve_identifier_token: DeployedValidatorRef,
    #[serde(rename(deserialize = "mintVECompositionToken"))]
    pub mint_ve_composition_token: DeployedValidatorRef,
    pub weighting_power: DeployedValidatorRef,
}

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BuiltPolicy {
    pub script: Script,
    pub policy_id: ScriptHash,
    pub asset_name: cml_chain::assets::AssetName,
    pub quantity: BigInteger,
}

#[derive(serde::Serialize, serde::Deserialize)]
pub struct MintedTokens {
    pub factory_auth: BuiltPolicy,
    pub wp_factory_auth: BuiltPolicy,
    pub ve_factory_auth: BuiltPolicy,
    pub perm_auth: BuiltPolicy,
    pub proposal_auth: BuiltPolicy,
    pub edao_msig: BuiltPolicy,
    pub ve_identifier: BuiltPolicy,
    pub inflation_auth: BuiltPolicy,
    pub gt: BuiltPolicy,
}

#[derive(serde::Deserialize)]
pub struct Deployment {
    pub validators: DeployedValidators,
    pub nfts: MintedTokens,
}

#[repr(u8)]
#[derive(Eq, PartialEq)]
pub enum ProtocolValidator {
    Inflation,
    VotingEscrow,
    SmartFarm,
    FarmFactory,
    WpFactory,
    VeFactory,
    GovProxy,
    PermManager,
    MintWpAuthPolicy,
    MintVeIdentifierToken,
    MintVeCompositionToken,
    WeightingPower,
}

#[derive(Debug, Copy, Clone)]
pub struct ProtocolScriptHashes {
    pub inflation: DeployedScriptInfo<{ ProtocolValidator::Inflation as u8 }>,
    pub voting_escrow: DeployedScriptInfo<{ ProtocolValidator::VotingEscrow as u8 }>,
    pub smart_farm: DeployedScriptInfo<{ ProtocolValidator::SmartFarm as u8 }>,
    pub farm_factory: DeployedScriptInfo<{ ProtocolValidator::FarmFactory as u8 }>,
    pub wp_factory: DeployedScriptInfo<{ ProtocolValidator::WpFactory as u8 }>,
    pub ve_factory: DeployedScriptInfo<{ ProtocolValidator::VeFactory as u8 }>,
    pub gov_proxy: DeployedScriptInfo<{ ProtocolValidator::GovProxy as u8 }>,
    pub perm_manager: DeployedScriptInfo<{ ProtocolValidator::PermManager as u8 }>,
    pub mint_wpauth_token: DeployedScriptInfo<{ ProtocolValidator::MintWpAuthPolicy as u8 }>,
    //pub mint_ve_identifier_token: DeployedScriptInfo<{ ProtocolValidator::MintVeIdentifierToken as u8 }>,
    pub mint_ve_composition_token: DeployedScriptInfo<{ ProtocolValidator::MintVeCompositionToken as u8 }>,
}

impl From<&ProtocolDeployment> for ProtocolScriptHashes {
    fn from(deployment: &ProtocolDeployment) -> Self {
        Self {
            inflation: DeployedScriptInfo::from(&deployment.inflation),
            voting_escrow: DeployedScriptInfo::from(&deployment.voting_escrow),
            smart_farm: DeployedScriptInfo::from(&deployment.smart_farm),
            farm_factory: DeployedScriptInfo::from(&deployment.farm_factory),
            wp_factory: DeployedScriptInfo::from(&deployment.wp_factory),
            ve_factory: DeployedScriptInfo::from(&deployment.ve_factory),
            gov_proxy: DeployedScriptInfo::from(&deployment.gov_proxy),
            perm_manager: DeployedScriptInfo::from(&deployment.perm_manager),
            mint_wpauth_token: DeployedScriptInfo::from(&deployment.mint_wpauth_token),
            // mint_ve_identifier_token: DeployedScriptInfo::from(&deployment.mint_ve_identifier_token),
            mint_ve_composition_token: DeployedScriptInfo::from(&deployment.mint_ve_composition_token),
        }
    }
}

#[derive(Debug, Clone)]
pub struct ProtocolDeployment {
    pub inflation: DeployedValidator<{ ProtocolValidator::Inflation as u8 }>,
    pub voting_escrow: DeployedValidator<{ ProtocolValidator::VotingEscrow as u8 }>,
    pub smart_farm: DeployedValidator<{ ProtocolValidator::SmartFarm as u8 }>,
    pub farm_factory: DeployedValidator<{ ProtocolValidator::FarmFactory as u8 }>,
    pub wp_factory: DeployedValidator<{ ProtocolValidator::WpFactory as u8 }>,
    pub ve_factory: DeployedValidator<{ ProtocolValidator::VeFactory as u8 }>,
    pub gov_proxy: DeployedValidator<{ ProtocolValidator::GovProxy as u8 }>,
    pub perm_manager: DeployedValidator<{ ProtocolValidator::PermManager as u8 }>,
    pub mint_wpauth_token: DeployedValidator<{ ProtocolValidator::MintWpAuthPolicy as u8 }>,
    //pub mint_ve_identifier_token: DeployedValidator<{ ProtocolValidator::MintVeIdentifierToken as u8 }>,
    pub mint_ve_composition_token: DeployedValidator<{ ProtocolValidator::MintVeCompositionToken as u8 }>,
    pub weighting_power: DeployedValidator<{ ProtocolValidator::WeightingPower as u8 }>,
}

impl ProtocolDeployment {
    pub async fn unsafe_pull<Net: CardanoNetwork>(validators: DeployedValidators, explorer: &Net) -> Self {
        Self {
            inflation: DeployedValidator::unsafe_pull(validators.inflation, explorer).await,
            voting_escrow: DeployedValidator::unsafe_pull(validators.voting_escrow, explorer).await,
            smart_farm: DeployedValidator::unsafe_pull(validators.smart_farm, explorer).await,
            farm_factory: DeployedValidator::unsafe_pull(validators.farm_factory, explorer).await,
            wp_factory: DeployedValidator::unsafe_pull(validators.wp_factory, explorer).await,
            ve_factory: DeployedValidator::unsafe_pull(validators.ve_factory, explorer).await,
            gov_proxy: DeployedValidator::unsafe_pull(validators.gov_proxy, explorer).await,
            perm_manager: DeployedValidator::unsafe_pull(validators.perm_manager, explorer).await,
            mint_wpauth_token: DeployedValidator::unsafe_pull(validators.mint_wpauth_token, explorer).await,
            //mint_ve_identifier_token: DeployedValidator::unsafe_pull(
            //    validators.mint_ve_identifier_token,
            //    explorer,
            //)
            //.await,
            mint_ve_composition_token: DeployedValidator::unsafe_pull(
                validators.mint_ve_composition_token,
                explorer,
            )
            .await,
            weighting_power: DeployedValidator::unsafe_pull(validators.weighting_power, explorer).await,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::Deployment;

    #[test]
    fn test_load_deployment() {
        let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        path.push("test_resources/preprod.deployment.json");
        println!("PATH: {:?}", path);
        let raw_deployment = std::fs::read_to_string(path).expect("Cannot load deployment file");
        let deployment: Deployment = serde_json::from_str(&raw_deployment).expect("Invalid deployment file");
    }
}
