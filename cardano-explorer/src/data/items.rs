use serde::Deserialize;

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
