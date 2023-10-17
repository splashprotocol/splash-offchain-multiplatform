use crate::data;
use serde::{Deserialize, Deserializer};
use std::marker::PhantomData;

#[derive(Deserialize)]
pub struct Items<T> {
    items: Vec<T>,
    total: u64,
}

impl<T> Items<T> {
    pub fn get_items(self) -> Vec<T> {
        self.items
    }
}
