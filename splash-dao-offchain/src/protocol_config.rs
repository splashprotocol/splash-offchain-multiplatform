use cml_chain::address::RewardAddress;
use cml_chain::builders::tx_builder::TransactionUnspentOutput;
use cml_chain::PolicyId;
use spectrum_cardano_lib::collateral::Collateral;

use crate::entities::onchain::inflation_box::InflationBoxId;
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
    pub factory_auth_policy: PolicyId,
    pub gt_policy: PolicyId,
    pub genesis_time: GenesisEpochStartTime,
}

impl ProtocolConfig {
    pub fn poll_id(&self, epoch: ProtocolEpoch) -> WeightingPollId {
        todo!("implement binder")
    }
}

pub const TX_FEE_CORRECTION: u64 = 1000;
