use spectrum_offchain::data::unique_entity::{Confirmed, Predicted, Traced, Unconfirmed};
use spectrum_offchain::data::LiquiditySource;

pub mod cache;
pub mod sources;

pub trait StateIndex<Src: LiquiditySource> {
    /// Get state id preceding given predicted state.
    fn get_prediction_predecessor<'a>(&self, id: Src::Version) -> Option<Src::Version>;
    /// Get last predicted state of the given entity.
    fn get_last_predicted<'a>(&self, id: Src::StableId) -> Option<Predicted<Src>>;
    /// Get last confirmed state of the given entity.
    fn get_last_confirmed<'a>(&self, id: Src::StableId) -> Option<Confirmed<Src>>;
    /// Get last unconfirmed state of the given entity.
    fn get_last_unconfirmed<'a>(&self, id: Src::StableId) -> Option<Unconfirmed<Src>>;
    /// Persist predicted state of the entity.
    fn put_predicted<'a>(&mut self, entity: Traced<Predicted<Src>>);
    /// Persist confirmed state of the entity.
    fn put_confirmed<'a>(&mut self, entity: Confirmed<Src>);
    /// Persist unconfirmed state of the entity.
    fn put_unconfirmed<'a>(&mut self, entity: Unconfirmed<Src>);
    /// Invalidate particular state of the entity.
    fn invalidate<'a>(&mut self, ver: Src::Version, id: Src::StableId);
    /// Invalidate particular state of the entity.
    fn eliminate<'a>(&mut self, ver: Src::Version, id: Src::StableId);
    /// False-positive analog of `exists()`.
    fn may_exist<'a>(&self, sid: Src::Version) -> bool;
    fn get_state<'a>(&self, sid: Src::Version) -> Option<Src>;
}
