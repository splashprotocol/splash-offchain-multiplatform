use cardano_explorer::CardanoNetwork;
use cml_chain::{plutus::ExUnits, utils::BigInteger};
use cml_crypto::{ScriptHash, TransactionHash};
use spectrum_cardano_lib::{NetworkId, Token};
use spectrum_offchain::domain::Has;
use spectrum_offchain_cardano::{
    deployment::{DeployedScriptInfo, DeployedValidator, DeployedValidatorRef, Script},
    has_deployed_script_info,
};
use tokio::io::AsyncWriteExt;
use type_equalities::IsEqual;

use crate::{
    constants::DAO_SCRIPT_BYTES,
    protocol_config::{GTAuthPolicy, SplashPolicy, VEFactoryAuthPolicy},
    GenesisEpochStartTime,
};

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
    pub make_ve_order: DeployedValidatorRef,
    pub extend_ve_order: DeployedValidatorRef,
    pub harvest_order: DeployedValidatorRef,
    pub redeem_ve_order: DeployedValidatorRef,
    pub wpoll_vote_order: DeployedValidatorRef,
    pub buffer_wallet: DeployedValidatorRef,
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
pub struct IssuedAsset {
    pub policy_id: ScriptHash,
    pub asset_name: cml_chain::assets::AssetName,
    pub quantity: BigInteger,
}

impl IssuedAsset {
    fn try_from_string(value: String) -> Option<Self> {
        let mut chunks = value.split(":");
        let Token(policy_id, asset_name) = Token::try_from_string(chunks.next()?)?;
        let quantity = chunks.next()?.parse().ok()?;
        Some(Self {
            policy_id,
            asset_name: asset_name.into(),
            quantity,
        })
    }
}

impl TryFrom<String> for IssuedAsset {
    type Error = String;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::try_from_string(value)
            .ok_or("IssuedAsset must be in format: policy_id.asset_name:quantity".to_string())
    }
}

#[derive(serde::Serialize, serde::Deserialize, Clone)]
pub struct ProtocolTokens {
    pub factory_auth: IssuedAsset,
    pub wp_factory_auth: IssuedAsset,
    pub ve_factory_auth: IssuedAsset,
    pub perm_auth: IssuedAsset,
    pub proposal_auth: IssuedAsset,
    pub edao_msig: IssuedAsset,
    pub inflation_auth: IssuedAsset,
    pub gt: IssuedAsset,
    pub buffer_wallet: IssuedAsset,
}

#[derive(serde::Deserialize)]
pub struct Deployment {
    pub validators: DeployedValidators,
    pub nfts: ProtocolTokens,
    pub script_bytes: DaoScriptData,
}

#[derive(serde::Deserialize, Clone, Debug)]
pub struct DaoScriptData {
    pub mint_weighting_power: TokenPolicyBytesAndCosts,
    pub inflation: ScriptBytesAndCosts,
    pub wp_factory: ScriptBytesAndCosts,
    pub mint_wp_auth_token: TokenPolicyBytesAndCosts,
    pub voting_escrow: ScriptBytesAndCosts,
    pub mint_farm_auth_token: ScriptBytesAndCosts,
    pub perm_manager: ScriptBytesAndCosts,
    pub one_time_mint: ScriptBytesAndCosts,
    pub mint_governance_power: ScriptBytesAndCosts,
    pub mint_identifier: ScriptBytesAndCosts,
    pub farm_factory: ScriptBytesAndCosts,
    pub ve_factory: ScriptBytesAndCosts,
    pub gov_proxy: ScriptBytesAndCosts,
    pub mint_ve_composition_token: ScriptBytesAndCosts,
    pub wpoll_vote_order: ScriptBytesAndCosts,
    pub make_voting_escrow_order: ScriptBytesAndCosts,
    pub extend_voting_escrow_order: ScriptBytesAndCosts,
    pub redeem_voting_escrow_order: ScriptBytesAndCosts,
    pub redeem_voting_escrow_witness: ScriptBytesAndCosts,
    pub proxy_order_witness: ScriptBytesAndCosts,
    pub harvest_order: ScriptBytesAndCosts,
    pub buffer_wallet: ScriptBytesAndCosts,
}

impl DaoScriptData {
    pub fn global() -> &'static DaoScriptData {
        DAO_SCRIPT_BYTES
            .get()
            .expect("DAO script bytes is not initialized")
    }
}

#[derive(serde::Deserialize, Clone, Debug)]
pub struct ScriptBytesAndCosts {
    /// Hex-encoded script bytes
    pub script_bytes: String,
    pub ex_units: ExUnits,
}

#[derive(serde::Deserialize, Clone, Debug)]
pub struct TokenPolicyBytesAndCosts {
    /// Hex-encoded script bytes
    pub script_bytes: String,
    pub mint_ex_units: ExUnits,
    pub burn_ex_units: ExUnits,
}

#[repr(u8)]
#[derive(Eq, PartialEq)]
pub enum ProtocolValidator {
    Inflation = 100,
    VotingEscrow = 101,
    SmartFarm = 102,
    FarmFactory = 103,
    WpFactory = 104,
    VeFactory = 105,
    GovProxy = 106,
    PermManager = 107,
    MintWpAuthPolicy = 108,
    MintIdentifier = 109,
    MintVeCompositionToken = 110,
    WeightingPower = 111,
    MakeVeOrder = 112,
    ExtendVeOrder = 113,
    HarvestOrder = 114,
    WPollVoteOrder = 115,
    RedeemVeOrder = 116,
    BufferWallet = 117,
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
    pub buffer_wallet: DeployedScriptInfo<{ ProtocolValidator::BufferWallet as u8 }>,
    pub make_ve_order: DeployedScriptInfo<{ ProtocolValidator::MakeVeOrder as u8 }>,
    pub extend_ve_order: DeployedScriptInfo<{ ProtocolValidator::ExtendVeOrder as u8 }>,
    pub harvest_order: DeployedScriptInfo<{ ProtocolValidator::HarvestOrder as u8 }>,
    pub wpoll_vote_order: DeployedScriptInfo<{ ProtocolValidator::WPollVoteOrder as u8 }>,
    pub redeem_ve_order: DeployedScriptInfo<{ ProtocolValidator::RedeemVeOrder as u8 }>,
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
            buffer_wallet: DeployedScriptInfo::from(&deployment.buffer_wallet),
            make_ve_order: DeployedScriptInfo::from(&deployment.make_ve_order),
            extend_ve_order: DeployedScriptInfo::from(&deployment.extend_ve_order),
            harvest_order: DeployedScriptInfo::from(&deployment.harvest_order),
            wpoll_vote_order: DeployedScriptInfo::from(&deployment.wpoll_vote_order),
            redeem_ve_order: DeployedScriptInfo::from(&deployment.redeem_ve_order),
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
    pub buffer_wallet: DeployedValidator<{ ProtocolValidator::BufferWallet as u8 }>,
    pub make_ve_order: DeployedValidator<{ ProtocolValidator::MakeVeOrder as u8 }>,
    pub extend_ve_order: DeployedValidator<{ ProtocolValidator::ExtendVeOrder as u8 }>,
    pub harvest_order: DeployedValidator<{ ProtocolValidator::HarvestOrder as u8 }>,
    pub wpoll_vote_order: DeployedValidator<{ ProtocolValidator::WPollVoteOrder as u8 }>,
    pub redeem_ve_order: DeployedValidator<{ ProtocolValidator::RedeemVeOrder as u8 }>,
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
            buffer_wallet: DeployedValidator::unsafe_pull(validators.buffer_wallet, explorer).await,
            make_ve_order: DeployedValidator::unsafe_pull(validators.make_ve_order, explorer).await,
            extend_ve_order: DeployedValidator::unsafe_pull(validators.extend_ve_order, explorer).await,
            harvest_order: DeployedValidator::unsafe_pull(validators.harvest_order, explorer).await,
            wpoll_vote_order: DeployedValidator::unsafe_pull(validators.wpoll_vote_order, explorer).await,
            redeem_ve_order: DeployedValidator::unsafe_pull(validators.redeem_ve_order, explorer).await,
        }
    }
}

#[derive(serde::Serialize, serde::Deserialize, Clone)]
pub struct DeploymentProgress {
    pub lq_tokens: Option<ExternallyMintedToken>,
    pub splash_tokens: Option<ExternallyMintedToken>,
    pub nft_utxo_inputs: Option<NFTUtxoInputs>,
    pub minted_deployment_tokens: Option<ProtocolTokens>,
    pub deployed_validators: Option<DeployedValidators>,
    pub genesis_epoch_start_time: Option<u64>,
    pub initial_farms: Vec<IssuedAsset>,
}

pub async fn write_deployment_to_disk(deployment_config: &DeploymentProgress, deployment_json_path: &str) {
    let mut file = tokio::fs::File::create(deployment_json_path).await.unwrap();
    file.write_all((serde_json::to_string(deployment_config).unwrap()).as_bytes())
        .await
        .unwrap();
}

#[derive(serde::Serialize, serde::Deserialize, Clone)]
pub struct CompleteDeployment {
    pub lq_tokens: ExternallyMintedToken,
    pub splash_tokens: ExternallyMintedToken,
    pub nft_utxo_inputs: NFTUtxoInputs,
    pub minted_deployment_tokens: ProtocolTokens,
    pub deployed_validators: DeployedValidators,
    pub genesis_epoch_start_time: u64,
    pub network_id: NetworkId,
    pub initial_farms: Vec<IssuedAsset>,
}

impl Has<VEFactoryAuthPolicy> for CompleteDeployment {
    fn select<U: IsEqual<VEFactoryAuthPolicy>>(&self) -> VEFactoryAuthPolicy {
        VEFactoryAuthPolicy(self.minted_deployment_tokens.ve_factory_auth.clone())
    }
}

impl Has<NetworkId> for CompleteDeployment {
    fn select<U: IsEqual<NetworkId>>(&self) -> NetworkId {
        self.network_id
    }
}

impl Has<GTAuthPolicy> for CompleteDeployment {
    fn select<U: IsEqual<GTAuthPolicy>>(&self) -> GTAuthPolicy {
        GTAuthPolicy(self.minted_deployment_tokens.gt.policy_id)
    }
}

impl Has<GenesisEpochStartTime> for CompleteDeployment {
    fn select<U: IsEqual<GenesisEpochStartTime>>(&self) -> GenesisEpochStartTime {
        GenesisEpochStartTime::from(self.genesis_epoch_start_time)
    }
}

impl Has<SplashPolicy> for CompleteDeployment {
    fn select<U: IsEqual<SplashPolicy>>(&self) -> SplashPolicy {
        SplashPolicy(self.splash_tokens.policy_id)
    }
}

use ProtocolValidator::*;

has_deployed_script_info!(
    MintVeCompositionToken,
    CompleteDeployment,
    |ctx: &CompleteDeployment| { (&ctx.deployed_validators.mint_ve_composition_token).into() }
);

has_deployed_script_info!(MintIdentifier, CompleteDeployment, |ctx: &CompleteDeployment| {
    (&ctx.deployed_validators.mint_identifier).into()
});

has_deployed_script_info!(
    MintWpAuthPolicy,
    CompleteDeployment,
    |ctx: &CompleteDeployment| { (&ctx.deployed_validators.mint_wpauth_token).into() }
);

has_deployed_script_info!(VeFactory, CompleteDeployment, |ctx: &CompleteDeployment| {
    (&ctx.deployed_validators.ve_factory).into()
});

has_deployed_script_info!(VotingEscrow, CompleteDeployment, |ctx: &CompleteDeployment| {
    (&ctx.deployed_validators.voting_escrow).into()
});

has_deployed_script_info!(ExtendVeOrder, CompleteDeployment, |ctx: &CompleteDeployment| {
    (&ctx.deployed_validators.extend_ve_order).into()
});

has_deployed_script_info!(WPollVoteOrder, CompleteDeployment, |ctx: &CompleteDeployment| {
    (&ctx.deployed_validators.wpoll_vote_order).into()
});

has_deployed_script_info!(RedeemVeOrder, CompleteDeployment, |ctx: &CompleteDeployment| {
    (&ctx.deployed_validators.redeem_ve_order).into()
});

impl TryFrom<(DeploymentProgress, NetworkId)> for CompleteDeployment {
    type Error = ();

    fn try_from((value, network_id): (DeploymentProgress, NetworkId)) -> Result<Self, Self::Error> {
        match value {
            DeploymentProgress {
                lq_tokens: Some(lq_tokens),
                splash_tokens: Some(splash_tokens),
                nft_utxo_inputs: Some(nft_utxo_inputs),
                minted_deployment_tokens: Some(minted_deployment_tokens),
                deployed_validators: Some(deployed_validators),
                genesis_epoch_start_time: Some(genesis_epoch_start_time),
                initial_farms,
            } => Ok(Self {
                lq_tokens,
                splash_tokens,
                nft_utxo_inputs,
                minted_deployment_tokens,
                deployed_validators,
                genesis_epoch_start_time,
                network_id,
                initial_farms,
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
