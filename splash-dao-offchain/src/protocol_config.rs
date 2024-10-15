use cardano_explorer::constants::get_network_id;
use cml_chain::address::{Address, EnterpriseAddress};
use cml_chain::assets::AssetName;
use cml_chain::builders::tx_builder::TransactionUnspentOutput;
use cml_chain::certs::StakeCredential;
use cml_chain::PolicyId;
use cml_crypto::{Bip32PrivateKey, Ed25519KeyHash, PrivateKey, ScriptHash};
use spectrum_cardano_lib::collateral::Collateral;
use spectrum_cardano_lib::{AssetClass, NetworkId, Token};
use spectrum_offchain::data::Has;
use spectrum_offchain_cardano::creds::operator_creds;
use spectrum_offchain_cardano::deployment::DeployedScriptInfo;
use std::ops::Index;
use type_equalities::IsEqual;

use crate::assets::SPLASH_AC;
use crate::deployment::{DeployedValidators, MintedTokens, ProtocolDeployment, ProtocolValidator};
use crate::entities::onchain::inflation_box::InflationBoxId;
use crate::entities::onchain::permission_manager::PermManagerId;
use crate::entities::onchain::poll_factory::PollFactoryId;
use crate::entities::onchain::weighting_poll::WeightingPollId;
use crate::time::ProtocolEpoch;
use crate::{CurrentEpoch, GenesisEpochStartTime};

#[derive(Clone)]
pub struct ProtocolConfig {
    pub deployed_validators: ProtocolDeployment,
    pub tokens: MintedTokens,
    pub operator_sk: String,
    pub node_magic: u64,
    pub network_id: NetworkId,
    pub reward_address: cml_chain::address::RewardAddress,
    pub collateral: Collateral,
    pub genesis_time: GenesisEpochStartTime,
}

impl ProtocolConfig {
    pub fn poll_id(&self, epoch: ProtocolEpoch) -> WeightingPollId {
        WeightingPollId(0)
    }
}

#[derive(Debug, Clone)]
pub struct InflationBoxRefScriptOutput(pub TransactionUnspentOutput);

#[derive(Debug, Clone)]
pub struct InflationAuthPolicy(pub PolicyId);

#[derive(Debug, Clone)]
pub struct Reward(pub cml_chain::address::RewardAddress);

#[derive(Debug, Clone)]
pub struct SplashPolicy(pub PolicyId);

#[derive(Debug, Clone)]
pub struct PollFactoryRefScriptOutput(pub TransactionUnspentOutput);

#[derive(Debug, Clone)]
pub struct MintWPAuthPolicy(pub PolicyId);

#[derive(Debug, Clone)]
pub struct MintWPAuthRefScriptOutput(pub TransactionUnspentOutput);

#[derive(Debug, Clone)]
pub struct MintVEIdentifierPolicy(pub PolicyId);

#[derive(Debug, Clone)]
pub struct MintVEIdentifierRefScriptOutput(pub TransactionUnspentOutput);

#[derive(Debug, Clone)]
pub struct MintVECompositionPolicy(pub PolicyId);

#[derive(Debug, Clone)]
pub struct MintVECompositionRefScriptOutput(pub TransactionUnspentOutput);

#[derive(Debug, Clone)]
pub struct FarmAuthPolicy(pub PolicyId);

#[derive(Debug, Clone)]
pub struct FarmAuthRefScriptOutput(pub TransactionUnspentOutput);

#[derive(Debug, Clone)]
pub struct FarmFactoryAuthPolicy(pub PolicyId);

#[derive(Debug, Clone)]
pub struct WPFactoryAuthPolicy(pub PolicyId);

#[derive(Debug, Clone)]
pub struct VEFactoryAuthPolicy(pub PolicyId);

#[derive(Debug, Clone)]
pub struct VotingEscrowRefScriptOutput(pub TransactionUnspentOutput);

#[derive(Debug, Clone)]
pub struct VotingEscrowPolicy(pub PolicyId);

#[derive(Debug, Clone)]
pub struct WeightingPowerPolicy(pub PolicyId);

#[derive(Debug, Clone)]
pub struct WeightingPowerRefScriptOutput(pub TransactionUnspentOutput);

#[derive(Debug, Clone)]
pub struct PermManagerBoxRefScriptOutput(pub TransactionUnspentOutput);

#[derive(Debug, Clone)]
pub struct GovProxyRefScriptOutput(pub TransactionUnspentOutput);

#[derive(Debug, Clone)]
pub struct EDaoMSigAuthPolicy(pub PolicyId);

#[derive(Debug, Clone)]
pub struct PermManagerAuthPolicy(pub PolicyId);

#[derive(Debug, Clone)]
pub struct GTAuthPolicy(pub PolicyId);

#[derive(Debug, Clone)]
pub struct NodeMagic(pub u64);

pub struct OperatorCreds(pub Ed25519KeyHash, pub Address);

pub trait NotOutputRefNorSlotNumber {}

impl NotOutputRefNorSlotNumber for OperatorCreds {}
impl NotOutputRefNorSlotNumber for CurrentEpoch {}
impl NotOutputRefNorSlotNumber for SplashPolicy {}
impl NotOutputRefNorSlotNumber for PermManagerAuthPolicy {}
impl NotOutputRefNorSlotNumber for MintWPAuthPolicy {}
impl NotOutputRefNorSlotNumber for MintVEIdentifierPolicy {}
impl NotOutputRefNorSlotNumber for MintVECompositionPolicy {}
impl NotOutputRefNorSlotNumber for VEFactoryAuthPolicy {}
impl NotOutputRefNorSlotNumber for GTAuthPolicy {}
impl<const TYP: u8> NotOutputRefNorSlotNumber for DeployedScriptInfo<TYP> {}

impl Has<Reward> for ProtocolConfig {
    fn select<U: IsEqual<Reward>>(&self) -> Reward {
        Reward(self.reward_address.clone())
    }
}

impl Has<Collateral> for ProtocolConfig {
    fn select<U: IsEqual<Collateral>>(&self) -> Collateral {
        self.collateral.clone()
    }
}

impl Has<SplashPolicy> for ProtocolConfig {
    fn select<U: IsEqual<SplashPolicy>>(&self) -> SplashPolicy {
        SplashPolicy(get_splash_token().0)
    }
}

impl Has<InflationBoxRefScriptOutput> for ProtocolConfig {
    fn select<U: IsEqual<InflationBoxRefScriptOutput>>(&self) -> InflationBoxRefScriptOutput {
        InflationBoxRefScriptOutput(self.deployed_validators.inflation.reference_utxo.clone())
    }
}

impl Has<InflationAuthPolicy> for ProtocolConfig {
    fn select<U: IsEqual<InflationAuthPolicy>>(&self) -> InflationAuthPolicy {
        InflationAuthPolicy(self.tokens.inflation_auth.policy_id)
    }
}

impl Has<PollFactoryRefScriptOutput> for ProtocolConfig {
    fn select<U: IsEqual<PollFactoryRefScriptOutput>>(&self) -> PollFactoryRefScriptOutput {
        PollFactoryRefScriptOutput(self.deployed_validators.wp_factory.reference_utxo.clone())
    }
}

impl Has<MintWPAuthPolicy> for ProtocolConfig {
    fn select<U: IsEqual<MintWPAuthPolicy>>(&self) -> MintWPAuthPolicy {
        MintWPAuthPolicy(self.deployed_validators.mint_wpauth_token.hash)
    }
}

impl Has<MintWPAuthRefScriptOutput> for ProtocolConfig {
    fn select<U: IsEqual<MintWPAuthRefScriptOutput>>(&self) -> MintWPAuthRefScriptOutput {
        MintWPAuthRefScriptOutput(self.deployed_validators.mint_wpauth_token.reference_utxo.clone())
    }
}

impl Has<MintVEIdentifierPolicy> for ProtocolConfig {
    fn select<U: IsEqual<MintVEIdentifierPolicy>>(&self) -> MintVEIdentifierPolicy {
        // MintVEIdentifierPolicy(self.deployed_validators.mint_ve_identifier_token.hash)
        todo!()
    }
}

impl Has<MintVEIdentifierRefScriptOutput> for ProtocolConfig {
    fn select<U: IsEqual<MintVEIdentifierRefScriptOutput>>(&self) -> MintVEIdentifierRefScriptOutput {
        //MintVEIdentifierRefScriptOutput(
        //    self.deployed_validators
        //        .mint_ve_identifier_token
        //        .reference_utxo
        //        .clone(),
        //)
        todo!()
    }
}

impl Has<MintVECompositionPolicy> for ProtocolConfig {
    fn select<U: IsEqual<MintVECompositionPolicy>>(&self) -> MintVECompositionPolicy {
        MintVECompositionPolicy(self.deployed_validators.mint_ve_composition_token.hash)
    }
}

impl Has<MintVECompositionRefScriptOutput> for ProtocolConfig {
    fn select<U: IsEqual<MintVECompositionRefScriptOutput>>(&self) -> MintVECompositionRefScriptOutput {
        MintVECompositionRefScriptOutput(
            self.deployed_validators
                .mint_ve_composition_token
                .reference_utxo
                .clone(),
        )
    }
}

impl Has<FarmAuthPolicy> for ProtocolConfig {
    fn select<U: IsEqual<FarmAuthPolicy>>(&self) -> FarmAuthPolicy {
        // Note that this policy is a multivalidator with `smart_farm`
        FarmAuthPolicy(self.deployed_validators.smart_farm.hash)
    }
}

impl Has<FarmAuthRefScriptOutput> for ProtocolConfig {
    fn select<U: IsEqual<FarmAuthRefScriptOutput>>(&self) -> FarmAuthRefScriptOutput {
        FarmAuthRefScriptOutput(self.deployed_validators.smart_farm.reference_utxo.clone())
    }
}

impl Has<FarmFactoryAuthPolicy> for ProtocolConfig {
    fn select<U: IsEqual<FarmFactoryAuthPolicy>>(&self) -> FarmFactoryAuthPolicy {
        FarmFactoryAuthPolicy(self.tokens.factory_auth.policy_id)
    }
}

impl Has<WPFactoryAuthPolicy> for ProtocolConfig {
    fn select<U: IsEqual<WPFactoryAuthPolicy>>(&self) -> WPFactoryAuthPolicy {
        WPFactoryAuthPolicy(self.tokens.wp_factory_auth.policy_id)
    }
}

impl Has<VEFactoryAuthPolicy> for ProtocolConfig {
    fn select<U: IsEqual<VEFactoryAuthPolicy>>(&self) -> VEFactoryAuthPolicy {
        VEFactoryAuthPolicy(self.tokens.ve_factory_auth.policy_id)
    }
}

impl Has<VotingEscrowRefScriptOutput> for ProtocolConfig {
    fn select<U: IsEqual<VotingEscrowRefScriptOutput>>(&self) -> VotingEscrowRefScriptOutput {
        VotingEscrowRefScriptOutput(self.deployed_validators.voting_escrow.reference_utxo.clone())
    }
}

impl Has<VotingEscrowPolicy> for ProtocolConfig {
    fn select<U: IsEqual<VotingEscrowPolicy>>(&self) -> VotingEscrowPolicy {
        VotingEscrowPolicy(self.deployed_validators.voting_escrow.hash)
    }
}

impl Has<WeightingPowerPolicy> for ProtocolConfig {
    fn select<U: IsEqual<WeightingPowerPolicy>>(&self) -> WeightingPowerPolicy {
        WeightingPowerPolicy(self.deployed_validators.weighting_power.hash)
    }
}

impl Has<WeightingPowerRefScriptOutput> for ProtocolConfig {
    fn select<U: IsEqual<WeightingPowerRefScriptOutput>>(&self) -> WeightingPowerRefScriptOutput {
        WeightingPowerRefScriptOutput(self.deployed_validators.weighting_power.reference_utxo.clone())
    }
}

impl Has<PermManagerBoxRefScriptOutput> for ProtocolConfig {
    fn select<U: IsEqual<PermManagerBoxRefScriptOutput>>(&self) -> PermManagerBoxRefScriptOutput {
        PermManagerBoxRefScriptOutput(self.deployed_validators.perm_manager.reference_utxo.clone())
    }
}

impl Has<EDaoMSigAuthPolicy> for ProtocolConfig {
    fn select<U: IsEqual<EDaoMSigAuthPolicy>>(&self) -> EDaoMSigAuthPolicy {
        todo!()
    }
}

impl Has<PermManagerAuthPolicy> for ProtocolConfig {
    fn select<U: IsEqual<PermManagerAuthPolicy>>(&self) -> PermManagerAuthPolicy {
        PermManagerAuthPolicy(self.tokens.perm_auth.policy_id)
    }
}

impl Has<GovProxyRefScriptOutput> for ProtocolConfig {
    fn select<U: IsEqual<GovProxyRefScriptOutput>>(&self) -> GovProxyRefScriptOutput {
        GovProxyRefScriptOutput(self.deployed_validators.gov_proxy.reference_utxo.clone())
    }
}

impl Has<GTAuthPolicy> for ProtocolConfig {
    fn select<U: IsEqual<GTAuthPolicy>>(&self) -> GTAuthPolicy {
        GTAuthPolicy(self.tokens.gt.policy_id)
    }
}

impl Has<GenesisEpochStartTime> for ProtocolConfig {
    fn select<U: IsEqual<GenesisEpochStartTime>>(&self) -> GenesisEpochStartTime {
        self.genesis_time
    }
}

impl Has<NodeMagic> for ProtocolConfig {
    fn select<U: IsEqual<NodeMagic>>(&self) -> NodeMagic {
        NodeMagic(self.node_magic)
    }
}

impl Has<OperatorCreds> for ProtocolConfig {
    fn select<U: IsEqual<OperatorCreds>>(&self) -> OperatorCreds {
        let (operator_cred, _, funding_addresses) = operator_creds(&self.operator_sk, self.network_id);
        OperatorCreds(operator_cred.0, funding_addresses.index(0).clone())
    }
}

impl Has<DeployedScriptInfo<{ ProtocolValidator::GovProxy as u8 }>> for ProtocolConfig {
    fn select<U: IsEqual<DeployedScriptInfo<{ ProtocolValidator::GovProxy as u8 }>>>(
        &self,
    ) -> DeployedScriptInfo<{ ProtocolValidator::GovProxy as u8 }> {
        DeployedScriptInfo::from(&self.deployed_validators.gov_proxy)
    }
}

impl Has<DeployedScriptInfo<{ ProtocolValidator::MintWpAuthPolicy as u8 }>> for ProtocolConfig {
    fn select<U: IsEqual<DeployedScriptInfo<{ ProtocolValidator::MintWpAuthPolicy as u8 }>>>(
        &self,
    ) -> DeployedScriptInfo<{ ProtocolValidator::MintWpAuthPolicy as u8 }> {
        DeployedScriptInfo::from(&self.deployed_validators.mint_wpauth_token)
    }
}

impl Has<DeployedScriptInfo<{ ProtocolValidator::MintIdentifier as u8 }>> for ProtocolConfig {
    fn select<U: IsEqual<DeployedScriptInfo<{ ProtocolValidator::MintIdentifier as u8 }>>>(
        &self,
    ) -> DeployedScriptInfo<{ ProtocolValidator::MintIdentifier as u8 }> {
        DeployedScriptInfo::from(&self.deployed_validators.mint_identifier)
    }
}

impl Has<DeployedScriptInfo<{ ProtocolValidator::MintVeCompositionToken as u8 }>> for ProtocolConfig {
    fn select<U: IsEqual<DeployedScriptInfo<{ ProtocolValidator::MintVeCompositionToken as u8 }>>>(
        &self,
    ) -> DeployedScriptInfo<{ ProtocolValidator::MintVeCompositionToken as u8 }> {
        DeployedScriptInfo::from(&self.deployed_validators.mint_ve_composition_token)
    }
}

impl Has<DeployedScriptInfo<{ ProtocolValidator::VotingEscrow as u8 }>> for ProtocolConfig {
    fn select<U: IsEqual<DeployedScriptInfo<{ ProtocolValidator::VotingEscrow as u8 }>>>(
        &self,
    ) -> DeployedScriptInfo<{ ProtocolValidator::VotingEscrow as u8 }> {
        DeployedScriptInfo::from(&self.deployed_validators.voting_escrow)
    }
}

impl Has<DeployedScriptInfo<{ ProtocolValidator::Inflation as u8 }>> for ProtocolConfig {
    fn select<U: IsEqual<DeployedScriptInfo<{ ProtocolValidator::Inflation as u8 }>>>(
        &self,
    ) -> DeployedScriptInfo<{ ProtocolValidator::Inflation as u8 }> {
        DeployedScriptInfo::from(&self.deployed_validators.inflation)
    }
}

impl Has<DeployedScriptInfo<{ ProtocolValidator::PermManager as u8 }>> for ProtocolConfig {
    fn select<U: IsEqual<DeployedScriptInfo<{ ProtocolValidator::PermManager as u8 }>>>(
        &self,
    ) -> DeployedScriptInfo<{ ProtocolValidator::PermManager as u8 }> {
        DeployedScriptInfo::from(&self.deployed_validators.perm_manager)
    }
}

impl Has<DeployedScriptInfo<{ ProtocolValidator::WpFactory as u8 }>> for ProtocolConfig {
    fn select<U: IsEqual<DeployedScriptInfo<{ ProtocolValidator::WpFactory as u8 }>>>(
        &self,
    ) -> DeployedScriptInfo<{ ProtocolValidator::WpFactory as u8 }> {
        DeployedScriptInfo::from(&self.deployed_validators.wp_factory)
    }
}

impl Has<DeployedScriptInfo<{ ProtocolValidator::SmartFarm as u8 }>> for ProtocolConfig {
    fn select<U: IsEqual<DeployedScriptInfo<{ ProtocolValidator::SmartFarm as u8 }>>>(
        &self,
    ) -> DeployedScriptInfo<{ ProtocolValidator::SmartFarm as u8 }> {
        DeployedScriptInfo::from(&self.deployed_validators.smart_farm)
    }
}

pub const TX_FEE_CORRECTION: u64 = 1000;

fn get_splash_token() -> (PolicyId, AssetName) {
    if let spectrum_cardano_lib::AssetClass::Token(Token(policy_id, name)) = *SPLASH_AC {
        (policy_id, AssetName::from(name))
    } else {
        panic!("Splash token can't be a native asset")
    }
}
