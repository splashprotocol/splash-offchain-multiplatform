use std::fmt::{Debug, Display, Formatter};

use derive_more::{From, Into};
use rand::RngCore;

#[derive(Debug, Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Into, From)]
pub struct Time(u64);

#[cfg(test)]
#[derive(Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Hash, From)]
pub struct StableId([u8; 28]);

#[cfg(test)]
impl StableId {
    #[cfg(test)]
    pub fn random() -> StableId {
        let mut bf = [0u8; 28];
        rand::thread_rng().fill_bytes(&mut bf);
        StableId(bf)
    }
}

#[cfg(test)]
impl Debug for StableId {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(&*hex::encode(&self.0))
    }
}

#[cfg(test)]
impl Display for StableId {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(&*hex::encode(&self.0))
    }
}
