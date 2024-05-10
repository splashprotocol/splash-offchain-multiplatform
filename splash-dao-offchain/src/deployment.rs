use cardano_explorer::client::Explorer;
use cml_chain::utils::BigInteger;
use cml_crypto::ScriptHash;
use spectrum_cardano_lib::AssetName;
use spectrum_offchain_cardano::deployment::{
    DeployedScriptHash, DeployedValidator, DeployedValidatorRef, Script,
};

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DaoDeployment {
    pub validators: DeployedValidators,
}

#[derive(serde::Deserialize)]
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
    pub weighting_power: DeployedValidatorRef,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BuiltPolicy {
    pub script: Script,
    pub policy_id: ScriptHash,
    pub asset_name: cml_chain::assets::AssetName,
    pub quantity: BigInteger,
}

#[derive(serde::Deserialize)]
pub struct MintedTokens {
    pub factory_auth: BuiltPolicy,
    pub ve_factory_auth: BuiltPolicy,
    pub perm_auth: BuiltPolicy,
    pub proposal_auth: BuiltPolicy,
    pub edao_msig: BuiltPolicy,
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
    WpAuthPolicy,
}

#[derive(Debug, Copy, Clone)]
pub struct ProtocolScriptHashes {
    pub inflation: DeployedScriptHash<{ ProtocolValidator::Inflation as u8 }>,
    pub voting_escrow: DeployedScriptHash<{ ProtocolValidator::VotingEscrow as u8 }>,
    pub smart_farm: DeployedScriptHash<{ ProtocolValidator::SmartFarm as u8 }>,
    pub farm_factory: DeployedScriptHash<{ ProtocolValidator::FarmFactory as u8 }>,
    pub wp_factory: DeployedScriptHash<{ ProtocolValidator::WpFactory as u8 }>,
    pub ve_factory: DeployedScriptHash<{ ProtocolValidator::VeFactory as u8 }>,
    pub gov_proxy: DeployedScriptHash<{ ProtocolValidator::GovProxy as u8 }>,
    pub perm_manager: DeployedScriptHash<{ ProtocolValidator::PermManager as u8 }>,
    pub mint_wpauth_token: DeployedScriptHash<{ ProtocolValidator::WpAuthPolicy as u8 }>,
}

impl From<&ProtocolDeployment> for ProtocolScriptHashes {
    fn from(deployment: &ProtocolDeployment) -> Self {
        Self {
            inflation: DeployedScriptHash::from(&deployment.inflation),
            voting_escrow: DeployedScriptHash::from(&deployment.voting_escrow),
            smart_farm: DeployedScriptHash::from(&deployment.smart_farm),
            farm_factory: DeployedScriptHash::from(&deployment.farm_factory),
            wp_factory: DeployedScriptHash::from(&deployment.wp_factory),
            ve_factory: DeployedScriptHash::from(&deployment.ve_factory),
            gov_proxy: DeployedScriptHash::from(&deployment.gov_proxy),
            perm_manager: DeployedScriptHash::from(&deployment.perm_manager),
            mint_wpauth_token: DeployedScriptHash::from(&deployment.mint_wpauth_token),
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
    pub mint_wpauth_token: DeployedValidator<{ ProtocolValidator::WpAuthPolicy as u8 }>,
}

impl ProtocolDeployment {
    pub async fn unsafe_pull<'a>(validators: DeployedValidators, explorer: &Explorer<'a>) -> Self {
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
