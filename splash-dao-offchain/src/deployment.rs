use cardano_explorer::CardanoNetwork;
use cml_chain::utils::BigInteger;
use cml_crypto::{ScriptHash, TransactionHash};
use spectrum_offchain::data::Has;
use spectrum_offchain_cardano::deployment::{
    DeployedScriptInfo, DeployedValidator, DeployedValidatorRef, Script,
};
use tokio::io::AsyncWriteExt;
use type_equalities::IsEqual;

use crate::protocol_config::{GTAuthPolicy, VEFactoryAuthPolicy};

#[derive(serde::Serialize, serde::Deserialize, Clone)]
pub struct DeployedValidators {
    pub inflation: DeployedValidatorRef,
    pub voting_escrow: DeployedValidatorRef,
    pub farm_factory: DeployedValidatorRef,
    pub wp_factory: DeployedValidatorRef,
    pub ve_factory: DeployedValidatorRef,
    pub gov_proxy: DeployedValidatorRef,
    pub perm_manager: DeployedValidatorRef,
    pub mint_wpauth_token: DeployedValidatorRef,
    pub mint_identifier: DeployedValidatorRef,
    pub mint_ve_composition_token: DeployedValidatorRef,
    pub weighting_power: DeployedValidatorRef,
    pub smart_farm: DeployedValidatorRef,
}

#[derive(serde::Serialize, serde::Deserialize, Clone)]
pub struct BuiltPolicy {
    pub policy_id: ScriptHash,
    pub asset_name: cml_chain::assets::AssetName,
    pub quantity: BigInteger,
}

#[derive(serde::Serialize, serde::Deserialize, Clone)]
pub struct MintedTokens {
    pub factory_auth: BuiltPolicy,
    pub wp_factory_auth: BuiltPolicy,
    pub ve_factory_auth: BuiltPolicy,
    pub perm_auth: BuiltPolicy,
    pub proposal_auth: BuiltPolicy,
    pub edao_msig: BuiltPolicy,
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
    MintIdentifier,
    MintVeCompositionToken,
    WeightingPower,
}

#[derive(Debug, Copy, Clone)]
pub struct ProtocolScriptHashes {
    pub inflation: DeployedScriptInfo<{ ProtocolValidator::Inflation as u8 }>,
    pub voting_escrow: DeployedScriptInfo<{ ProtocolValidator::VotingEscrow as u8 }>,
    pub farm_factory: DeployedScriptInfo<{ ProtocolValidator::FarmFactory as u8 }>,
    pub wp_factory: DeployedScriptInfo<{ ProtocolValidator::WpFactory as u8 }>,
    pub ve_factory: DeployedScriptInfo<{ ProtocolValidator::VeFactory as u8 }>,
    pub gov_proxy: DeployedScriptInfo<{ ProtocolValidator::GovProxy as u8 }>,
    pub perm_manager: DeployedScriptInfo<{ ProtocolValidator::PermManager as u8 }>,
    pub mint_wpauth_token: DeployedScriptInfo<{ ProtocolValidator::MintWpAuthPolicy as u8 }>,
    pub mint_identifier: DeployedScriptInfo<{ ProtocolValidator::MintIdentifier as u8 }>,
    pub mint_ve_composition_token: DeployedScriptInfo<{ ProtocolValidator::MintVeCompositionToken as u8 }>,
    pub weighting_power: DeployedScriptInfo<{ ProtocolValidator::WeightingPower as u8 }>,
    pub smart_farm: DeployedScriptInfo<{ ProtocolValidator::SmartFarm as u8 }>,
}

impl From<&ProtocolDeployment> for ProtocolScriptHashes {
    fn from(deployment: &ProtocolDeployment) -> Self {
        Self {
            inflation: DeployedScriptInfo::from(&deployment.inflation),
            voting_escrow: DeployedScriptInfo::from(&deployment.voting_escrow),
            farm_factory: DeployedScriptInfo::from(&deployment.farm_factory),
            wp_factory: DeployedScriptInfo::from(&deployment.wp_factory),
            ve_factory: DeployedScriptInfo::from(&deployment.ve_factory),
            gov_proxy: DeployedScriptInfo::from(&deployment.gov_proxy),
            perm_manager: DeployedScriptInfo::from(&deployment.perm_manager),
            mint_wpauth_token: DeployedScriptInfo::from(&deployment.mint_wpauth_token),
            mint_identifier: DeployedScriptInfo::from(&deployment.mint_identifier),
            mint_ve_composition_token: DeployedScriptInfo::from(&deployment.mint_ve_composition_token),
            weighting_power: DeployedScriptInfo::from(&deployment.weighting_power),
            smart_farm: DeployedScriptInfo::from(&deployment.smart_farm),
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
    pub mint_wpauth_token: DeployedValidator<{ ProtocolValidator::MintWpAuthPolicy as u8 }>,
    pub mint_identifier: DeployedValidator<{ ProtocolValidator::MintIdentifier as u8 }>,
    pub mint_ve_composition_token: DeployedValidator<{ ProtocolValidator::MintVeCompositionToken as u8 }>,
    pub weighting_power: DeployedValidator<{ ProtocolValidator::WeightingPower as u8 }>,
    pub smart_farm: DeployedValidator<{ ProtocolValidator::SmartFarm as u8 }>,
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
            mint_identifier: DeployedValidator::unsafe_pull(validators.mint_identifier, explorer).await,
            mint_ve_composition_token: DeployedValidator::unsafe_pull(
                validators.mint_ve_composition_token,
                explorer,
            )
            .await,
            weighting_power: DeployedValidator::unsafe_pull(validators.weighting_power, explorer).await,
        }
    }
}

#[derive(serde::Serialize, serde::Deserialize)]
pub struct DeploymentProgress {
    pub lq_tokens: Option<ExternallyMintedToken>,
    pub splash_tokens: Option<ExternallyMintedToken>,
    pub nft_utxo_inputs: Option<NFTUtxoInputs>,
    pub minted_deployment_tokens: Option<MintedTokens>,
    pub deployed_validators: Option<DeployedValidators>,
}

pub async fn write_deployment_to_disk(deployment_config: &DeploymentProgress, deployment_json_path: &str) {
    let mut file = tokio::fs::File::create(deployment_json_path).await.unwrap();
    file.write_all((serde_json::to_string(deployment_config).unwrap()).as_bytes())
        .await
        .unwrap();
}

pub struct CompleteDeployment {
    pub lq_tokens: ExternallyMintedToken,
    pub splash_tokens: ExternallyMintedToken,
    pub nft_utxo_inputs: NFTUtxoInputs,
    pub minted_deployment_tokens: MintedTokens,
    pub deployed_validators: DeployedValidators,
}

impl Has<VEFactoryAuthPolicy> for CompleteDeployment {
    fn select<U: IsEqual<VEFactoryAuthPolicy>>(&self) -> VEFactoryAuthPolicy {
        VEFactoryAuthPolicy(self.minted_deployment_tokens.ve_factory_auth.policy_id)
    }
}

impl Has<GTAuthPolicy> for CompleteDeployment {
    fn select<U: IsEqual<GTAuthPolicy>>(&self) -> GTAuthPolicy {
        GTAuthPolicy(self.minted_deployment_tokens.gt.policy_id)
    }
}

impl Has<DeployedScriptInfo<{ ProtocolValidator::VeFactory as u8 }>> for CompleteDeployment {
    fn select<U: IsEqual<DeployedScriptInfo<{ ProtocolValidator::VeFactory as u8 }>>>(
        &self,
    ) -> DeployedScriptInfo<{ ProtocolValidator::VeFactory as u8 }> {
        DeployedScriptInfo::from(&self.deployed_validators.ve_factory)
    }
}

impl TryFrom<DeploymentProgress> for CompleteDeployment {
    type Error = ();

    fn try_from(value: DeploymentProgress) -> Result<Self, Self::Error> {
        match value {
            DeploymentProgress {
                lq_tokens: Some(lq_tokens),
                splash_tokens: Some(splash_tokens),
                nft_utxo_inputs: Some(nft_utxo_inputs),
                minted_deployment_tokens: Some(minted_deployment_tokens),
                deployed_validators: Some(deployed_validators),
            } => Ok(Self {
                lq_tokens,
                splash_tokens,
                nft_utxo_inputs,
                minted_deployment_tokens,
                deployed_validators,
            }),
            _ => Err(()),
        }
    }
}

#[derive(serde::Serialize, serde::Deserialize, Clone)]
pub struct ExternallyMintedToken {
    pub policy_id: ScriptHash,
    pub asset_name: cml_chain::assets::AssetName,
    pub quantity: u64,
}

#[derive(serde::Serialize, serde::Deserialize, Clone)]
/// Each NFT we mint requires a distinct UTxO input.
pub struct NFTUtxoInputs {
    pub tx_hash: TransactionHash,
    pub number_of_inputs: usize,
    pub inputs_consumed: bool,
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
