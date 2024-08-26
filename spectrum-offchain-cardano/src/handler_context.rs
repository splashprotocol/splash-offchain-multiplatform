use derive_more::{From, Into};
use spectrum_cardano_lib::OutputRef;
use spectrum_offchain::small_set::SmallVec;

#[derive(Debug, Copy, Clone, Into, From, Default)]
pub struct ConsumedInputs(pub SmallVec<OutputRef>);

#[derive(Debug, Copy, Clone, Into, From)]
pub struct ConsumedIdentifiers<I: Copy>(pub SmallVec<I>);

impl<I: Copy> Default for ConsumedIdentifiers<I> {
    fn default() -> Self {
        Self(SmallVec::default())
    }
}

#[derive(Debug, Copy, Clone, Into, From)]
pub struct ProducedIdentifiers<I: Copy>(pub SmallVec<I>);

impl<I: Copy> Default for ProducedIdentifiers<I> {
    fn default() -> Self {
        Self(SmallVec::default())
    }
}
