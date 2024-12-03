use std::collections::HashMap;
use std::fmt::Display;
use std::hash::Hash;

use log::trace;
use type_equalities::IsEqual;

use spectrum_offchain::maker::Maker;

#[derive(Debug, Clone)]
pub struct MultiPair<PairId, R, Ctx>(HashMap<PairId, R>, Ctx, &'static str);

impl<PairId, R, Ctx> MultiPair<PairId, R, Ctx> {
    pub fn new<Hint: IsEqual<R>>(context: Ctx, tag: &'static str) -> Self {
        Self(HashMap::new(), context, tag)
    }
}

impl<PairId, R, Ctx> MultiPair<PairId, R, Ctx>
where
    PairId: Copy + Eq + Hash + Display,
    R: Maker<PairId, Ctx>,
    Ctx: Clone,
{
    pub fn with_resource_mut<F, T>(&mut self, pair: &PairId, f: F) -> T
    where
        F: FnOnce(&mut R) -> T,
    {
        f(self.get_mut(pair))
    }

    pub fn get_mut(&mut self, pair: &PairId) -> &mut R {
        if self.0.contains_key(pair) {
            self.0.get_mut(pair).unwrap()
        } else {
            trace!(target: "offchain", "MultiPair[{}]: new pair: {}", self.2, pair);
            self.0.insert(*pair, Maker::make(*pair, &self.1));
            self.get_mut(pair)
        }
    }

    pub fn remove(&mut self, pair: &PairId) {
        self.0.remove(pair);
    }
}
