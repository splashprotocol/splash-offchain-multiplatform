use spectrum_offchain::data::LiquiditySource;

/// Entity bundled with its source.
#[derive(Debug, Clone, Eq, PartialEq)]
pub struct Bundled<T, Source>(pub T, pub Source);

impl<T, Source> LiquiditySource for Bundled<T, Source>
where
    T: LiquiditySource,
{
    type StableId = T::StableId;
    type Version = T::Version;
    fn stable_id(&self) -> Self::StableId {
        self.0.stable_id()
    }
    fn version(&self) -> Self::Version {
        self.0.version()
    }
}
