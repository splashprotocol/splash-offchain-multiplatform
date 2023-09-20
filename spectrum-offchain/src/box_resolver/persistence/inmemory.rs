use std::collections::HashMap;

use async_trait::async_trait;

use crate::box_resolver::persistence::EntityRepo;
use crate::data::unique_entity::{Confirmed, Predicted, Traced, Unconfirmed};
use crate::data::OnChainEntity;

type InMemoryKey = [u8; 33];

#[derive(Debug)]
pub struct InMemoryEntityRepo {
    store: HashMap<InMemoryKey, Vec<u8>>,
}

impl InMemoryEntityRepo {
    pub fn new() -> Self {
        Self {
            store: HashMap::new(),
        }
    }
}

#[async_trait(?Send)]
impl<T> EntityRepo<T> for InMemoryEntityRepo
where
    T: OnChainEntity + Clone + Send + 'static,
    <T as OnChainEntity>::TStateId: Clone + Send + 'static,
    <T as OnChainEntity>::TEntityId: Clone + Send + 'static,
{
    async fn get_prediction_predecessor<'a>(&self, id: T::TStateId) -> Option<T::TStateId>
    where
        <T as OnChainEntity>::TStateId: 'a,
    {
        todo!()
    }

    async fn get_last_predicted<'a>(&self, id: T::TEntityId) -> Option<Predicted<T>>
    where
        <T as OnChainEntity>::TEntityId: 'a,
    {
        todo!()
    }

    async fn get_last_confirmed<'a>(&self, id: T::TEntityId) -> Option<Confirmed<T>>
    where
        <T as OnChainEntity>::TEntityId: 'a,
    {
        todo!()
    }

    async fn get_last_unconfirmed<'a>(&self, id: T::TEntityId) -> Option<Unconfirmed<T>>
    where
        <T as OnChainEntity>::TEntityId: 'a,
    {
        todo!()
    }

    async fn put_predicted<'a>(&mut self, entity: Traced<Predicted<T>>)
    where
        Traced<Predicted<T>>: 'a,
    {
        todo!()
    }

    async fn put_confirmed<'a>(&mut self, entity: Confirmed<T>)
    where
        Traced<Predicted<T>>: 'a,
    {
        todo!()
    }

    async fn put_unconfirmed<'a>(&mut self, entity: Unconfirmed<T>)
    where
        Traced<Predicted<T>>: 'a,
    {
        todo!()
    }

    async fn invalidate<'a>(&mut self, sid: T::TStateId, eid: T::TEntityId)
    where
        <T as OnChainEntity>::TStateId: 'a,
        <T as OnChainEntity>::TEntityId: 'a,
    {
        todo!()
    }

    async fn eliminate<'a>(&mut self, entity: T)
    where
        T: 'a,
    {
        todo!()
    }

    async fn may_exist<'a>(&self, sid: T::TStateId) -> bool
    where
        <T as OnChainEntity>::TStateId: 'a,
    {
        todo!()
    }

    async fn get_state<'a>(&self, sid: T::TStateId) -> Option<T>
    where
        <T as OnChainEntity>::TStateId: 'a,
    {
        todo!()
    }
}
