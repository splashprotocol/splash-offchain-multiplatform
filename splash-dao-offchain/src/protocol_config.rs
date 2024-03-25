use cml_chain::address::RewardAddress;
use cml_chain::builders::tx_builder::TransactionUnspentOutput;
use cml_chain::PolicyId;
use spectrum_cardano_lib::collateral::Collateral;

use crate::entities::onchain::inflation_box::InflationBoxId;
use crate::entities::onchain::permission_manager::PermManagerId;
use crate::entities::onchain::poll_factory::PollFactoryId;
use crate::entities::onchain::weighting_poll::WeightingPollId;
use crate::time::ProtocolEpoch;
use crate::GenesisEpochStartTime;

pub struct ProtocolConfig {
    pub operator_sk: String,
    pub node_magic: u64,
    pub reward_address: RewardAddress,
    pub collateral: Collateral,
    pub splash_policy: PolicyId,
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
    pub voting_escrow_ref_script: TransactionUnspentOutput,
    pub weighting_power_ref_script: TransactionUnspentOutput,
    pub perm_manager_box_id: PermManagerId,
    pub perm_manager_box_ref_script: TransactionUnspentOutput,
    pub edao_msig_policy: PolicyId,
    pub perm_manager_auth_policy: PolicyId,
    pub gt_policy: PolicyId,
    pub genesis_time: GenesisEpochStartTime,
}

impl ProtocolConfig {
    pub fn poll_id(&self, epoch: ProtocolEpoch) -> WeightingPollId {
        todo!("implement binder")
    }
}

pub const TX_FEE_CORRECTION: u64 = 1000;
