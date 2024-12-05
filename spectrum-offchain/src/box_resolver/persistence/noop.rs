use async_trait::async_trait;

use crate::box_resolver::persistence::EntityRepo;
use crate::domain::event::{Confirmed, Predicted, Traced, Unconfirmed};
use crate::domain::{EntitySnapshot, Stable};

#[derive(Debug)]
pub struct NoopEntityRepo;

#[async_trait]
impl<T> EntityRepo<T> for NoopEntityRepo
where
    T: EntitySnapshot + Clone + Send + 'static,
    <T as EntitySnapshot>::Version: Clone + Send + 'static,
    <T as Stable>::StableId: Clone + Send + 'static,
{
    async fn get_prediction_predecessor<'a>(&self, _id: T::Version) -> Option<T::Version>
    where
        <T as EntitySnapshot>::Version: 'a,
    {
        None
    }

    async fn get_last_predicted<'a>(&self, _id: T::StableId) -> Option<Predicted<T>>
    where
        <T as Stable>::StableId: 'a,
    {
        None
    }

    async fn get_last_confirmed<'a>(&self, _id: T::StableId) -> Option<Confirmed<T>>
    where
        <T as Stable>::StableId: 'a,
    {
        None
    }

    async fn get_last_unconfirmed<'a>(&self, _id: T::StableId) -> Option<Unconfirmed<T>>
    where
        <T as Stable>::StableId: 'a,
    {
        None
    }

    async fn put_predicted<'a>(&mut self, _entity: Traced<Predicted<T>>)
    where
        Traced<Predicted<T>>: 'a,
    {
    }

    async fn put_confirmed<'a>(&mut self, _entity: Confirmed<T>)
    where
        Traced<Predicted<T>>: 'a,
    {
    }

    async fn put_unconfirmed<'a>(&mut self, _entity: Unconfirmed<T>)
    where
        Traced<Predicted<T>>: 'a,
    {
    }

    async fn invalidate<'a>(&mut self, _sid: T::Version, _eid: T::StableId)
    where
        <T as EntitySnapshot>::Version: 'a,
        <T as Stable>::StableId: 'a,
    {
    }

    async fn eliminate<'a>(&mut self, _entity: T)
    where
        T: 'a,
    {
    }

    async fn may_exist<'a>(&self, _sid: T::Version) -> bool
    where
        <T as EntitySnapshot>::Version: 'a,
    {
        false
    }

    async fn get_state<'a>(&self, _sid: T::Version) -> Option<T>
    where
        <T as EntitySnapshot>::Version: 'a,
    {
        None
    }
}
