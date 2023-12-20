use std::collections::HashSet;

use async_trait::async_trait;

use crate::data::LiquiditySource;

#[async_trait(?Send)]
pub trait EntityBlacklist<T: LiquiditySource> {
    async fn is_blacklisted(&self, id: &T::StableId) -> bool;
}

pub struct StaticBlacklist<T: LiquiditySource> {
    entries: HashSet<T::StableId>,
}

impl<T: LiquiditySource> StaticBlacklist<T> {
    pub fn new(entries: HashSet<T::StableId>) -> Self {
        Self { entries }
    }
}

#[async_trait(?Send)]
impl<T> EntityBlacklist<T> for StaticBlacklist<T>
where
    T: LiquiditySource,
{
    async fn is_blacklisted(&self, id: &T::StableId) -> bool {
        self.entries.contains(id)
    }
}
