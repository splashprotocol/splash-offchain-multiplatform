use splash_dao_offchain::entities::onchain::smart_farm::FarmId;

#[derive(Debug, Clone, PartialEq)]
pub struct SmartFarm<StateId> {
    pub state_id: StateId,
    pub farm_id: FarmId,
}