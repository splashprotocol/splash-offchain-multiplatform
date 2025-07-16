#[derive(Debug, Clone, PartialEq)]
pub struct Gauge<GaugeId, StateId> {
    pub id: GaugeId,
    pub state_id: StateId,
}
