use std::collections::HashSet;

use async_trait::async_trait;

use crate::data::OnChainEntity;

#[async_trait(?Send)]
pub trait EntityBlacklist<T: OnChainEntity> {
    async fn is_blacklisted(&self, id: &T::TEntityId) -> bool;
}

pub struct StaticBlacklist<T: OnChainEntity> {
    entries: HashSet<T::TEntityId>,
}

impl<T: OnChainEntity> StaticBlacklist<T> {
    pub fn new(entries: HashSet<T::TEntityId>) -> Self {
        Self { entries }
    }
}

#[async_trait(?Send)]
impl<T> EntityBlacklist<T> for StaticBlacklist<T>
where
    T: OnChainEntity,
{
    async fn is_blacklisted(&self, id: &T::TEntityId) -> bool {
        self.entries.contains(id)
    }
}
