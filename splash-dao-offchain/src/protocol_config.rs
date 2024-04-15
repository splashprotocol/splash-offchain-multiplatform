use cml_chain::address::Address;
use cml_chain::assets::AssetName;
use cml_chain::builders::tx_builder::TransactionUnspentOutput;
use cml_chain::PolicyId;
use cml_crypto::{Ed25519KeyHash, PrivateKey};
use spectrum_cardano_lib::collateral::Collateral;
use spectrum_offchain::data::Has;
use spectrum_offchain_cardano::creds::operator_creds;
use type_equalities::IsEqual;

use crate::entities::onchain::inflation_box::InflationBoxId;
use crate::entities::onchain::permission_manager::PermManagerId;
use crate::entities::onchain::poll_factory::PollFactoryId;
use crate::entities::onchain::weighting_poll::WeightingPollId;
use crate::time::ProtocolEpoch;
use crate::GenesisEpochStartTime;

pub struct ProtocolConfig {
    pub operator_sk: String,
    pub node_magic: u64,
    pub reward_address: cml_chain::address::RewardAddress,
    pub collateral: Collateral,
    pub splash_policy: PolicyId,
    pub splash_name: AssetName,
    pub inflation_box_id: InflationBoxId,
    pub inflation_box_ref_script: TransactionUnspentOutput,
    pub poll_factory_id: PollFactoryId,
    pub poll_factory_ref_script: TransactionUnspentOutput,
    pub wpoll_auth_policy: PolicyId,
    pub wpoll_auth_ref_script: TransactionUnspentOutput,
    pub farm_auth_policy: PolicyId,
    pub farm_auth_ref_script: TransactionUnspentOutput,
    pub factory_auth_policy: PolicyId,
    pub ve_factory_auth_policy: PolicyId,
    pub ve_factory_auth_name: AssetName,
    pub voting_escrow_ref_script: TransactionUnspentOutput,
    pub weighting_power_ref_script: TransactionUnspentOutput,
    pub perm_manager_box_id: PermManagerId,
    pub perm_manager_box_ref_script: TransactionUnspentOutput,
    pub edao_msig_policy: PolicyId,
    pub perm_manager_auth_policy: PolicyId,
    pub perm_manager_auth_name: AssetName,
    pub gov_proxy_ref_script: TransactionUnspentOutput,
    pub gt_policy: PolicyId,
    pub gt_name: AssetName,
    pub genesis_time: GenesisEpochStartTime,
}

impl ProtocolConfig {
    pub fn poll_id(&self, epoch: ProtocolEpoch) -> WeightingPollId {
        todo!("implement binder")
    }
}

#[derive(Debug, Clone)]
pub struct InflationBoxRefScriptOutput(pub TransactionUnspentOutput);

#[derive(Debug, Clone)]
pub struct Reward(pub cml_chain::address::RewardAddress);

#[derive(Debug, Clone)]
pub struct SplashPolicy(pub PolicyId);

#[derive(Debug, Clone)]
pub struct SplashAssetName(pub AssetName);

#[derive(Debug, Clone)]
pub struct PollFactoryRefScriptOutput(pub TransactionUnspentOutput);

#[derive(Debug, Clone)]
pub struct WPAuthPolicy(pub PolicyId);

#[derive(Debug, Clone)]
pub struct WPAuthRefScriptOutput(pub TransactionUnspentOutput);

#[derive(Debug, Clone)]
pub struct FarmAuthPolicy(pub PolicyId);

#[derive(Debug, Clone)]
pub struct FarmAuthRefScriptOutput(pub TransactionUnspentOutput);

#[derive(Debug, Clone)]
pub struct FactoryAuthPolicy(pub PolicyId);

#[derive(Debug, Clone)]
pub struct VEFactoryAuthPolicy(pub PolicyId);

#[derive(Debug, Clone)]
pub struct VEFactoryAuthName(pub AssetName);

#[derive(Debug, Clone)]
pub struct VotingEscrowRefScriptOutput(pub TransactionUnspentOutput);

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
pub struct PermManagerAuthName(pub AssetName);

#[derive(Debug, Clone)]
pub struct GTAuthPolicy(pub PolicyId);

#[derive(Debug, Clone)]
pub struct GTAuthName(pub AssetName);

#[derive(Debug, Clone)]
pub struct NodeMagic(pub u64);

pub struct OperatorCreds(pub PrivateKey, pub Ed25519KeyHash, pub Address);

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
        SplashPolicy(self.splash_policy)
    }
}

impl Has<SplashAssetName> for ProtocolConfig {
    fn select<U: IsEqual<SplashAssetName>>(&self) -> SplashAssetName {
        SplashAssetName(self.splash_name.clone())
    }
}

impl Has<InflationBoxRefScriptOutput> for ProtocolConfig {
    fn select<U: IsEqual<InflationBoxRefScriptOutput>>(&self) -> InflationBoxRefScriptOutput {
        InflationBoxRefScriptOutput(self.inflation_box_ref_script.clone())
    }
}

impl Has<PollFactoryRefScriptOutput> for ProtocolConfig {
    fn select<U: IsEqual<PollFactoryRefScriptOutput>>(&self) -> PollFactoryRefScriptOutput {
        PollFactoryRefScriptOutput(self.poll_factory_ref_script.clone())
    }
}

impl Has<WPAuthPolicy> for ProtocolConfig {
    fn select<U: IsEqual<WPAuthPolicy>>(&self) -> WPAuthPolicy {
        WPAuthPolicy(self.wpoll_auth_policy)
    }
}

impl Has<WPAuthRefScriptOutput> for ProtocolConfig {
    fn select<U: IsEqual<WPAuthRefScriptOutput>>(&self) -> WPAuthRefScriptOutput {
        WPAuthRefScriptOutput(self.wpoll_auth_ref_script.clone())
    }
}

impl Has<FarmAuthPolicy> for ProtocolConfig {
    fn select<U: IsEqual<FarmAuthPolicy>>(&self) -> FarmAuthPolicy {
        FarmAuthPolicy(self.farm_auth_policy)
    }
}

impl Has<FarmAuthRefScriptOutput> for ProtocolConfig {
    fn select<U: IsEqual<FarmAuthRefScriptOutput>>(&self) -> FarmAuthRefScriptOutput {
        FarmAuthRefScriptOutput(self.farm_auth_ref_script.clone())
    }
}

impl Has<FactoryAuthPolicy> for ProtocolConfig {
    fn select<U: IsEqual<FactoryAuthPolicy>>(&self) -> FactoryAuthPolicy {
        FactoryAuthPolicy(self.factory_auth_policy)
    }
}

impl Has<VEFactoryAuthPolicy> for ProtocolConfig {
    fn select<U: IsEqual<VEFactoryAuthPolicy>>(&self) -> VEFactoryAuthPolicy {
        VEFactoryAuthPolicy(self.ve_factory_auth_policy)
    }
}

impl Has<VEFactoryAuthName> for ProtocolConfig {
    fn select<U: IsEqual<VEFactoryAuthName>>(&self) -> VEFactoryAuthName {
        VEFactoryAuthName(self.ve_factory_auth_name)
    }
}

impl Has<VotingEscrowRefScriptOutput> for ProtocolConfig {
    fn select<U: IsEqual<VotingEscrowRefScriptOutput>>(&self) -> VotingEscrowRefScriptOutput {
        VotingEscrowRefScriptOutput(self.voting_escrow_ref_script.clone())
    }
}

impl Has<WeightingPowerRefScriptOutput> for ProtocolConfig {
    fn select<U: IsEqual<WeightingPowerRefScriptOutput>>(&self) -> WeightingPowerRefScriptOutput {
        WeightingPowerRefScriptOutput(self.weighting_power_ref_script.clone())
    }
}

impl Has<PermManagerBoxRefScriptOutput> for ProtocolConfig {
    fn select<U: IsEqual<PermManagerBoxRefScriptOutput>>(&self) -> PermManagerBoxRefScriptOutput {
        PermManagerBoxRefScriptOutput(self.perm_manager_box_ref_script.clone())
    }
}

impl Has<EDaoMSigAuthPolicy> for ProtocolConfig {
    fn select<U: IsEqual<EDaoMSigAuthPolicy>>(&self) -> EDaoMSigAuthPolicy {
        EDaoMSigAuthPolicy(self.edao_msig_policy)
    }
}

impl Has<PermManagerAuthPolicy> for ProtocolConfig {
    fn select<U: IsEqual<PermManagerAuthPolicy>>(&self) -> PermManagerAuthPolicy {
        PermManagerAuthPolicy(self.perm_manager_auth_policy)
    }
}

impl Has<PermManagerAuthName> for ProtocolConfig {
    fn select<U: IsEqual<PermManagerAuthName>>(&self) -> PermManagerAuthName {
        PermManagerAuthName(self.perm_manager_auth_name.clone())
    }
}

impl Has<GovProxyRefScriptOutput> for ProtocolConfig {
    fn select<U: IsEqual<GovProxyRefScriptOutput>>(&self) -> GovProxyRefScriptOutput {
        GovProxyRefScriptOutput(self.gov_proxy_ref_script.clone())
    }
}

impl Has<GTAuthPolicy> for ProtocolConfig {
    fn select<U: IsEqual<GTAuthPolicy>>(&self) -> GTAuthPolicy {
        GTAuthPolicy(self.gt_policy)
    }
}

impl Has<GTAuthName> for ProtocolConfig {
    fn select<U: IsEqual<GTAuthName>>(&self) -> GTAuthName {
        GTAuthName(self.gt_name)
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
        let (operator_sk, operator_pkh, operator_addr) = operator_creds(&self.operator_sk, self.node_magic);
        OperatorCreds(operator_sk, operator_pkh, operator_addr)
    }
}

pub const TX_FEE_CORRECTION: u64 = 1000;
