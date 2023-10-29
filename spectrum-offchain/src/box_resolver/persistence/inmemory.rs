use std::collections::HashMap;
use std::fmt::Debug;

use async_trait::async_trait;
use log::warn;

use crate::box_resolver::persistence::EntityRepo;
use crate::data::unique_entity::{Confirmed, Predicted, Traced, Unconfirmed};
use crate::data::OnChainEntity;

#[derive(Debug)]
pub struct InMemoryEntityRepo<T: OnChainEntity> {
    store: HashMap<T::Version, T>,
    index: HashMap<InMemoryIndexKey, T::Version>,
    links: HashMap<T::Version, T::Version>,
}

impl<T: OnChainEntity> InMemoryEntityRepo<T> {
    pub fn new() -> Self {
        Self {
            store: HashMap::new(),
            links: HashMap::new(),
            index: HashMap::new(),
        }
    }
}

type InMemoryIndexKey = [u8; 61];

const STATE_PREFIX: u8 = 0u8;
const PREDICTION_LINK_PREFIX: u8 = 1u8;
const LAST_PREDICTED_PREFIX: u8 = 2u8;
const LAST_CONFIRMED_PREFIX: u8 = 3u8;
const LAST_UNCONFIRMED_PREFIX: u8 = 4u8;

#[async_trait(?Send)]
impl<T> EntityRepo<T> for InMemoryEntityRepo<T>
where
    T: OnChainEntity + Clone + Send + 'static,
    <T as OnChainEntity>::Version: Copy + Send + Debug + 'static,
    <T as OnChainEntity>::Id: Copy + Send + Into<[u8; 60]> + 'static,
{
    async fn get_prediction_predecessor<'a>(&self, id: T::Version) -> Option<T::Version>
    where
        <T as OnChainEntity>::Version: 'a,
    {
        self.links.get(&id).map(|id| *id)
    }

    async fn get_last_predicted<'a>(&self, id: T::Id) -> Option<Predicted<T>>
    where
        <T as OnChainEntity>::Id: 'a,
    {
        let index_key = index_key(LAST_PREDICTED_PREFIX, id);
        self.index
            .get(&index_key)
            .and_then(|sid| self.store.get(sid))
            .map(|e| Predicted(e.clone()))
    }

    async fn get_last_confirmed<'a>(&self, id: T::Id) -> Option<Confirmed<T>>
    where
        <T as OnChainEntity>::Id: 'a,
    {
        let index_key = index_key(LAST_CONFIRMED_PREFIX, id);
        self.index
            .get(&index_key)
            .and_then(|sid| self.store.get(sid))
            .map(|e| Confirmed(e.clone()))
    }

    async fn get_last_unconfirmed<'a>(&self, id: T::Id) -> Option<Unconfirmed<T>>
    where
        <T as OnChainEntity>::Id: 'a,
    {
        let index_key = index_key(LAST_UNCONFIRMED_PREFIX, id);
        self.index
            .get(&index_key)
            .and_then(|sid| self.store.get(sid))
            .map(|e| Unconfirmed(e.clone()))
    }

    async fn put_predicted<'a>(
        &mut self,
        Traced {
            state: Predicted(entity),
            prev_state_id,
        }: Traced<Predicted<T>>,
    ) where
        Traced<Predicted<T>>: 'a,
    {
        let index_key = index_key(LAST_PREDICTED_PREFIX, entity.get_id());
        self.index.insert(index_key, entity.get_version());
        if let Some(prev_sid) = prev_state_id {
            self.links.insert(entity.get_version(), prev_sid);
        }
        self.store.insert(entity.get_version(), entity);
    }

    async fn put_confirmed<'a>(&mut self, Confirmed(entity): Confirmed<T>)
    where
        Traced<Predicted<T>>: 'a,
    {
        let index_key = index_key(LAST_CONFIRMED_PREFIX, entity.get_id());
        self.index.insert(index_key, entity.get_version());
        self.store.insert(entity.get_version(), entity);
    }

    async fn put_unconfirmed<'a>(&mut self, Unconfirmed(entity): Unconfirmed<T>)
    where
        Traced<Predicted<T>>: 'a,
    {
        let index_key = index_key(LAST_UNCONFIRMED_PREFIX, entity.get_id());
        self.index.insert(index_key, entity.get_version());
        self.store.insert(entity.get_version(), entity);
    }

    async fn invalidate<'a>(&mut self, sid: T::Version, eid: T::Id)
    where
        <T as OnChainEntity>::Version: 'a,
        <T as OnChainEntity>::Id: 'a,
    {
        let predecessor = self.get_prediction_predecessor(sid).await;
        let last_predicted_index_key = index_key(LAST_PREDICTED_PREFIX, eid);
        let last_confirmed_index_key = index_key(LAST_CONFIRMED_PREFIX, eid);
        let last_unconfirmed_index_key = index_key(LAST_UNCONFIRMED_PREFIX, eid);
        if let Some(predecessor) = predecessor {
            warn!(target: "entity_repo", "invalidating entity: rollback to {:?}", predecessor);
            warn!(target: "entity_repo", "invalidating entity: rollback to {:?}", predecessor);
            self.index.insert(last_confirmed_index_key, predecessor);
        } else {
            self.index.remove(&last_confirmed_index_key);
        }
        self.index.remove(&last_predicted_index_key);
        self.index.remove(&last_unconfirmed_index_key);
        self.links.remove(&sid);
        self.store.remove(&sid);
    }

    async fn eliminate<'a>(&mut self, entity: T)
    where
        T: 'a,
    {
        let eid = entity.get_id();
        let sid = entity.get_version();
        let last_predicted_index_key = index_key(LAST_PREDICTED_PREFIX, eid);
        let last_confirmed_index_key = index_key(LAST_CONFIRMED_PREFIX, eid);
        let last_unconfirmed_index_key = index_key(LAST_UNCONFIRMED_PREFIX, eid);
        self.index.remove(&last_predicted_index_key);
        self.index.remove(&last_confirmed_index_key);
        self.index.remove(&last_unconfirmed_index_key);
        self.links.remove(&sid);
        self.store.remove(&sid);
    }

    async fn may_exist<'a>(&self, sid: T::Version) -> bool
    where
        <T as OnChainEntity>::Version: 'a,
    {
        self.store.contains_key(&sid)
    }

    async fn get_state<'a>(&self, sid: T::Version) -> Option<T>
    where
        <T as OnChainEntity>::Version: 'a,
    {
        self.store.get(&sid).map(|e| e.clone())
    }
}

pub fn index_key<T: Into<[u8; 60]>>(prefix: u8, id: T) -> InMemoryIndexKey {
    let mut arr = [prefix; 61];
    let raw_id: [u8; 60] = id.into();
    for (ix, byte) in raw_id.into_iter().enumerate() {
        arr[ix + 1] = byte;
    }
    arr
}
