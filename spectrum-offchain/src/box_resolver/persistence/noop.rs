use async_trait::async_trait;

use crate::box_resolver::persistence::EntityRepo;
use crate::data::unique_entity::{Confirmed, Predicted, Traced, Unconfirmed};
use crate::data::OnChainEntity;

#[derive(Debug)]
pub struct NoopEntityRepo;

#[async_trait(?Send)]
impl<T> EntityRepo<T> for NoopEntityRepo
where
    T: OnChainEntity + Clone + Send + 'static,
    <T as OnChainEntity>::Version: Clone + Send + 'static,
    <T as OnChainEntity>::Id: Clone + Send + 'static,
{
    async fn get_prediction_predecessor<'a>(&self, id: T::Version) -> Option<T::Version>
    where
        <T as OnChainEntity>::Version: 'a,
    {
        None
    }

    async fn get_last_predicted<'a>(&self, id: T::Id) -> Option<Predicted<T>>
    where
        <T as OnChainEntity>::Id: 'a,
    {
        None
    }

    async fn get_last_confirmed<'a>(&self, id: T::Id) -> Option<Confirmed<T>>
    where
        <T as OnChainEntity>::Id: 'a,
    {
        None
    }

    async fn get_last_unconfirmed<'a>(&self, id: T::Id) -> Option<Unconfirmed<T>>
    where
        <T as OnChainEntity>::Id: 'a,
    {
        None
    }

    async fn put_predicted<'a>(&mut self, entity: Traced<Predicted<T>>)
    where
        Traced<Predicted<T>>: 'a,
    {
    }

    async fn put_confirmed<'a>(&mut self, entity: Confirmed<T>)
    where
        Traced<Predicted<T>>: 'a,
    {
    }

    async fn put_unconfirmed<'a>(&mut self, entity: Unconfirmed<T>)
    where
        Traced<Predicted<T>>: 'a,
    {
    }

    async fn invalidate<'a>(&mut self, sid: T::Version, eid: T::Id)
    where
        <T as OnChainEntity>::Version: 'a,
        <T as OnChainEntity>::Id: 'a,
    {
    }

    async fn eliminate<'a>(&mut self, entity: T)
    where
        T: 'a,
    {
    }

    async fn may_exist<'a>(&self, sid: T::Version) -> bool
    where
        <T as OnChainEntity>::Version: 'a,
    {
        false
    }

    async fn get_state<'a>(&self, sid: T::Version) -> Option<T>
    where
        <T as OnChainEntity>::Version: 'a,
    {
        None
    }
}
