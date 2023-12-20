use std::fmt::{Debug, Formatter};

use rand::{thread_rng, RngCore};

#[derive(Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct StableId([u8; 32]);

impl StableId {
    #[cfg(test)]
    pub fn random() -> StableId {
        let mut bf = [0u8; 32];
        thread_rng().fill_bytes(&mut bf);
        StableId(bf)
    }
}

impl Debug for StableId {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(&*hex::encode(&self.0))
    }
}
