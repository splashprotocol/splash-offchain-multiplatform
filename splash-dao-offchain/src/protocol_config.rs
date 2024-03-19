use cml_chain::PolicyId;

use crate::entities::onchain::inflation_box::InflationBoxId;
use crate::entities::onchain::poll_factory::PollFactoryId;
use crate::entities::onchain::weighting_poll::WeightingPollId;
use crate::time::ProtocolEpoch;
use crate::GenesisEpochStartTime;

pub struct ProtocolConfig {
    pub inflation_box_id: InflationBoxId,
    pub poll_factory_id: PollFactoryId,
    pub wpoll_auth_policy: PolicyId,
    pub farm_auth_policy: PolicyId,
    pub gt_policy: PolicyId,
    pub genesis_time: GenesisEpochStartTime,
}

impl ProtocolConfig {
    pub fn poll_id(&self, epoch: ProtocolEpoch) -> WeightingPollId {
        todo!("implement binder")
    }
}
