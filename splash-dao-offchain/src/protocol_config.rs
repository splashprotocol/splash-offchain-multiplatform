use cml_chain::address::Address;
use cml_chain::builders::tx_builder::TransactionUnspentOutput;
use cml_chain::transaction::NativeScript;
use cml_chain::PolicyId;
use cml_crypto::{Ed25519KeyHash, ScriptHash};
use spectrum_cardano_lib::collateral::Collateral;
use spectrum_cardano_lib::NetworkId;
use spectrum_offchain::domain::Has;
use spectrum_offchain_cardano::creds::operator_creds;
use spectrum_offchain_cardano::deployment::DeployedScriptInfo;
use spectrum_offchain_cardano::{has_deployed_script_info, has_deployed_validator};
use std::ops::Index;
use type_equalities::IsEqual;

use crate::deployment::{IssuedAsset, ProtocolDeployment, ProtocolTokens, ProtocolValidator::*};
use crate::entities::onchain::weighting_poll::WeightingPollId;
use crate::time::ProtocolEpoch;
use crate::GenesisEpochStartTime;

#[derive(Clone)]
pub struct ProtocolConfig {
    pub deployed_validators: ProtocolDeployment,
    pub tokens: ProtocolTokens,
    pub operator_sk: String,
    pub node_magic: u64,
    pub network_id: NetworkId,
    pub reward_address: cml_chain::address::RewardAddress,
    pub splash_policy_id: PolicyId,
    pub collateral: Collateral,
    pub genesis_time: GenesisEpochStartTime,
}

impl ProtocolConfig {
    pub fn poll_id(&self, epoch: ProtocolEpoch) -> WeightingPollId {
        WeightingPollId(epoch)
    }
}

#[derive(Debug, Clone)]
pub struct InflationAuthPolicy(pub PolicyId);

#[derive(Debug, Clone)]
pub struct Reward(pub cml_chain::address::RewardAddress);

#[derive(Debug, Clone)]
pub struct SplashPolicy(pub PolicyId);

#[derive(Debug, Clone)]
pub struct MintWPAuthRefScriptOutput(pub TransactionUnspentOutput);

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
pub struct VEFactoryAuthPolicy(pub IssuedAsset);

#[derive(Debug, Clone)]
pub struct WPollVoteOrderScriptHash(pub ScriptHash);

#[derive(Debug, Clone)]
pub struct WPollVoteOrderRefScriptOutput(pub TransactionUnspentOutput);

#[derive(Debug, Clone)]
pub struct ExtendVotingEscrowOrderScriptHash(pub ScriptHash);

#[derive(Debug, Clone)]
pub struct ExtendVotingEscrowOrderRefScriptOutput(pub TransactionUnspentOutput);

#[derive(Debug, Clone)]
pub struct RedeemVotingEscrowOrderScriptHash(pub ScriptHash);

#[derive(Debug, Clone)]
pub struct RedeemVotingEscrowOrderRefScriptOutput(pub TransactionUnspentOutput);

#[derive(Debug, Clone)]
pub struct VotingEscrowRefScriptOutput(pub TransactionUnspentOutput);

#[derive(Debug, Clone)]
pub struct VotingEscrowScriptHash(pub PolicyId);

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
pub struct GTBuiltPolicy(pub IssuedAsset);

#[derive(Debug, Clone)]
pub struct BufferWalletScript(pub NativeScript);

#[derive(Debug, Clone)]
pub struct NodeMagic(pub u64);

#[derive(Clone)]
pub struct OperatorCreds(pub Ed25519KeyHash, pub Address);

pub trait NotOutputRefNorSlotNumber {}

impl NotOutputRefNorSlotNumber for OperatorCreds {}
impl NotOutputRefNorSlotNumber for SplashPolicy {}
impl NotOutputRefNorSlotNumber for FarmAuthPolicy {}
impl NotOutputRefNorSlotNumber for InflationAuthPolicy {}
impl NotOutputRefNorSlotNumber for WPFactoryAuthPolicy {}
impl NotOutputRefNorSlotNumber for PermManagerAuthPolicy {}
impl NotOutputRefNorSlotNumber for MintVECompositionPolicy {}
impl NotOutputRefNorSlotNumber for VEFactoryAuthPolicy {}
impl NotOutputRefNorSlotNumber for GenesisEpochStartTime {}
impl NotOutputRefNorSlotNumber for GTAuthPolicy {}
impl NotOutputRefNorSlotNumber for GTBuiltPolicy {}
impl NotOutputRefNorSlotNumber for NetworkId {}
impl<const TYP: u8> NotOutputRefNorSlotNumber for DeployedScriptInfo<TYP> {}

impl Has<NetworkId> for ProtocolConfig {
    fn select<U: IsEqual<NetworkId>>(&self) -> NetworkId {
        self.network_id
    }
}

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
        SplashPolicy(self.splash_policy_id)
    }
}

impl Has<InflationAuthPolicy> for ProtocolConfig {
    fn select<U: IsEqual<InflationAuthPolicy>>(&self) -> InflationAuthPolicy {
        InflationAuthPolicy(self.tokens.inflation_auth.policy_id)
    }
}

impl Has<MintWPAuthRefScriptOutput> for ProtocolConfig {
    fn select<U: IsEqual<MintWPAuthRefScriptOutput>>(&self) -> MintWPAuthRefScriptOutput {
        MintWPAuthRefScriptOutput(self.deployed_validators.mint_wpauth_token.reference_utxo.clone())
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
        VEFactoryAuthPolicy(self.tokens.ve_factory_auth.clone())
    }
}

impl Has<WPollVoteOrderScriptHash> for ProtocolConfig {
    fn select<U: IsEqual<WPollVoteOrderScriptHash>>(&self) -> WPollVoteOrderScriptHash {
        WPollVoteOrderScriptHash(self.deployed_validators.wpoll_vote_order.hash)
    }
}

impl Has<WPollVoteOrderRefScriptOutput> for ProtocolConfig {
    fn select<U: IsEqual<WPollVoteOrderRefScriptOutput>>(&self) -> WPollVoteOrderRefScriptOutput {
        WPollVoteOrderRefScriptOutput(self.deployed_validators.wpoll_vote_order.reference_utxo.clone())
    }
}

impl Has<ExtendVotingEscrowOrderScriptHash> for ProtocolConfig {
    fn select<U: IsEqual<ExtendVotingEscrowOrderScriptHash>>(&self) -> ExtendVotingEscrowOrderScriptHash {
        ExtendVotingEscrowOrderScriptHash(self.deployed_validators.extend_ve_order.hash)
    }
}

impl Has<ExtendVotingEscrowOrderRefScriptOutput> for ProtocolConfig {
    fn select<U: IsEqual<ExtendVotingEscrowOrderRefScriptOutput>>(
        &self,
    ) -> ExtendVotingEscrowOrderRefScriptOutput {
        ExtendVotingEscrowOrderRefScriptOutput(
            self.deployed_validators.extend_ve_order.reference_utxo.clone(),
        )
    }
}

impl Has<RedeemVotingEscrowOrderScriptHash> for ProtocolConfig {
    fn select<U: IsEqual<RedeemVotingEscrowOrderScriptHash>>(&self) -> RedeemVotingEscrowOrderScriptHash {
        RedeemVotingEscrowOrderScriptHash(self.deployed_validators.redeem_ve_order.hash)
    }
}

impl Has<RedeemVotingEscrowOrderRefScriptOutput> for ProtocolConfig {
    fn select<U: IsEqual<RedeemVotingEscrowOrderRefScriptOutput>>(
        &self,
    ) -> RedeemVotingEscrowOrderRefScriptOutput {
        RedeemVotingEscrowOrderRefScriptOutput(
            self.deployed_validators.redeem_ve_order.reference_utxo.clone(),
        )
    }
}

impl Has<VotingEscrowRefScriptOutput> for ProtocolConfig {
    fn select<U: IsEqual<VotingEscrowRefScriptOutput>>(&self) -> VotingEscrowRefScriptOutput {
        VotingEscrowRefScriptOutput(self.deployed_validators.voting_escrow.reference_utxo.clone())
    }
}

impl Has<VotingEscrowScriptHash> for ProtocolConfig {
    fn select<U: IsEqual<VotingEscrowScriptHash>>(&self) -> VotingEscrowScriptHash {
        VotingEscrowScriptHash(self.deployed_validators.voting_escrow.hash)
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
        EDaoMSigAuthPolicy(self.tokens.edao_msig.policy_id)
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

impl Has<GTBuiltPolicy> for ProtocolConfig {
    fn select<U: IsEqual<GTBuiltPolicy>>(&self) -> GTBuiltPolicy {
        GTBuiltPolicy(self.tokens.gt.clone())
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

has_deployed_validator!(GovProxy, ProtocolConfig, |ctx: &ProtocolConfig| ctx
    .deployed_validators
    .gov_proxy
    .clone());

has_deployed_script_info!(GovProxy, ProtocolConfig, |ctx: &ProtocolConfig| {
    (&ctx.deployed_validators.gov_proxy).into()
});

has_deployed_validator!(MintWpAuthPolicy, ProtocolConfig, |ctx: &ProtocolConfig| ctx
    .deployed_validators
    .mint_wpauth_token
    .clone());

has_deployed_script_info!(MintWpAuthPolicy, ProtocolConfig, |ctx: &ProtocolConfig| {
    (&ctx.deployed_validators.mint_wpauth_token).into()
});

has_deployed_validator!(MintIdentifier, ProtocolConfig, |ctx: &ProtocolConfig| ctx
    .deployed_validators
    .mint_identifier
    .clone());

has_deployed_script_info!(MintIdentifier, ProtocolConfig, |ctx: &ProtocolConfig| {
    (&ctx.deployed_validators.mint_identifier).into()
});

has_deployed_validator!(MintVeCompositionToken, ProtocolConfig, |ctx: &ProtocolConfig| ctx
    .deployed_validators
    .mint_ve_composition_token
    .clone());

has_deployed_script_info!(MintVeCompositionToken, ProtocolConfig, |ctx: &ProtocolConfig| {
    (&ctx.deployed_validators.mint_ve_composition_token).into()
});

has_deployed_validator!(VotingEscrow, ProtocolConfig, |ctx: &ProtocolConfig| ctx
    .deployed_validators
    .voting_escrow
    .clone());

has_deployed_script_info!(VotingEscrow, ProtocolConfig, |ctx: &ProtocolConfig| (&ctx
    .deployed_validators
    .voting_escrow)
    .into());

has_deployed_validator!(Inflation, ProtocolConfig, |ctx: &ProtocolConfig| ctx
    .deployed_validators
    .inflation
    .clone());

has_deployed_script_info!(Inflation, ProtocolConfig, |ctx: &ProtocolConfig| (&ctx
    .deployed_validators
    .inflation)
    .into());

has_deployed_validator!(PermManager, ProtocolConfig, |ctx: &ProtocolConfig| ctx
    .deployed_validators
    .perm_manager
    .clone());

has_deployed_script_info!(PermManager, ProtocolConfig, |ctx: &ProtocolConfig| (&ctx
    .deployed_validators
    .perm_manager)
    .into());

has_deployed_validator!(WpFactory, ProtocolConfig, |ctx: &ProtocolConfig| ctx
    .deployed_validators
    .wp_factory
    .clone());

has_deployed_script_info!(WpFactory, ProtocolConfig, |ctx: &ProtocolConfig| (&ctx
    .deployed_validators
    .wp_factory)
    .into());

has_deployed_validator!(SmartFarm, ProtocolConfig, |ctx: &ProtocolConfig| ctx
    .deployed_validators
    .smart_farm
    .clone());

has_deployed_script_info!(SmartFarm, ProtocolConfig, |ctx: &ProtocolConfig| (&ctx
    .deployed_validators
    .smart_farm)
    .into());

has_deployed_validator!(VeFactory, ProtocolConfig, |ctx: &ProtocolConfig| ctx
    .deployed_validators
    .ve_factory
    .clone());

has_deployed_script_info!(VeFactory, ProtocolConfig, |ctx: &ProtocolConfig| (&ctx
    .deployed_validators
    .ve_factory)
    .into());

has_deployed_validator!(MakeVeOrder, ProtocolConfig, |ctx: &ProtocolConfig| ctx
    .deployed_validators
    .make_ve_order
    .clone());

has_deployed_script_info!(MakeVeOrder, ProtocolConfig, |ctx: &ProtocolConfig| (&ctx
    .deployed_validators
    .make_ve_order)
    .into());

has_deployed_validator!(ExtendVeOrder, ProtocolConfig, |ctx: &ProtocolConfig| ctx
    .deployed_validators
    .extend_ve_order
    .clone());

has_deployed_script_info!(ExtendVeOrder, ProtocolConfig, |ctx: &ProtocolConfig| (&ctx
    .deployed_validators
    .extend_ve_order)
    .into());

has_deployed_validator!(WPollVoteOrder, ProtocolConfig, |ctx: &ProtocolConfig| ctx
    .deployed_validators
    .wpoll_vote_order
    .clone());

has_deployed_script_info!(WPollVoteOrder, ProtocolConfig, |ctx: &ProtocolConfig| (&ctx
    .deployed_validators
    .wpoll_vote_order)
    .into());

has_deployed_validator!(RedeemVeOrder, ProtocolConfig, |ctx: &ProtocolConfig| ctx
    .deployed_validators
    .redeem_ve_order
    .clone());

has_deployed_script_info!(RedeemVeOrder, ProtocolConfig, |ctx: &ProtocolConfig| (&ctx
    .deployed_validators
    .redeem_ve_order)
    .into());

impl Has<BufferWalletScript> for ProtocolConfig {
    fn select<U: IsEqual<BufferWalletScript>>(&self) -> BufferWalletScript {
        BufferWalletScript(self.deployed_validators.buffer_wallet.clone())
    }
}

pub const TX_FEE_CORRECTION: u64 = 1000;
