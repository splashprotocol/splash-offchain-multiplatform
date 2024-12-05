use std::collections::HashSet;

use async_trait::async_trait;

use crate::domain::EntitySnapshot;

#[async_trait(?Send)]
pub trait EntityBlacklist<T: EntitySnapshot> {
    async fn is_blacklisted(&self, id: &T::StableId) -> bool;
}

pub struct StaticBlacklist<T: EntitySnapshot> {
    entries: HashSet<T::StableId>,
}

impl<T: EntitySnapshot> StaticBlacklist<T> {
    pub fn new(entries: HashSet<T::StableId>) -> Self {
        Self { entries }
    }
}

#[async_trait(?Send)]
impl<T> EntityBlacklist<T> for StaticBlacklist<T>
where
    T: EntitySnapshot,
{
    async fn is_blacklisted(&self, id: &T::StableId) -> bool {
        self.entries.contains(id)
    }
}
