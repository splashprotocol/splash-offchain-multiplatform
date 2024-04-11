use cardano_explorer::client::Explorer;
use spectrum_offchain_cardano::deployment::{DeployedScriptHash, DeployedValidator, DeployedValidatorRef};

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeployedValidators {
    pub inflation: DeployedValidatorRef,
    pub voting_escrow: DeployedValidatorRef,
    pub farm_factory: DeployedValidatorRef,
    pub wp_factory: DeployedValidatorRef,
    pub ve_factory: DeployedValidatorRef,
    pub gov_proxy: DeployedValidatorRef,
    pub perm_manager: DeployedValidatorRef,
    pub mint_wpauth_token: DeployedValidatorRef,
}

#[repr(u8)]
#[derive(Eq, PartialEq)]
pub enum ProtocolValidator {
    Inflation,
    VotingEscrow,
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
            farm_factory: DeployedValidator::unsafe_pull(validators.farm_factory, explorer).await,
            wp_factory: DeployedValidator::unsafe_pull(validators.wp_factory, explorer).await,
            ve_factory: DeployedValidator::unsafe_pull(validators.ve_factory, explorer).await,
            gov_proxy: DeployedValidator::unsafe_pull(validators.gov_proxy, explorer).await,
            perm_manager: DeployedValidator::unsafe_pull(validators.perm_manager, explorer).await,
            mint_wpauth_token: DeployedValidator::unsafe_pull(validators.mint_wpauth_token, explorer).await,
        }
    }
}
