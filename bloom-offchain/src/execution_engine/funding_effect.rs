pub type FundingEff<T> = FundingEvent<T>;

#[derive(Clone)]
pub enum FundingEvent<T> {
    Consumed(T),
    Produced(T),
}

impl<T> FundingEvent<T> {
    pub fn inverse(self) -> FundingEvent<T> {
        match self {
            FundingEvent::Consumed(x) => FundingEvent::Produced(x),
            FundingEvent::Produced(x) => FundingEvent::Consumed(x),
        }
    }
}

pub enum FundingIO<T, K> {
    /// Old [Funding] is replaced with a new one.
    Replaced(T, K),
    /// Old [Funding] wasn't used, plus a new one is created.
    Added(T, K),
    /// Old [Funding] wasn't used.
    NotUsed(T),
}

impl<T, K> FundingIO<T, K> {
    pub fn map_output<F, B>(self, f: F) -> FundingIO<T, B>
    where
        F: FnOnce(K) -> B,
    {
        match self {
            FundingIO::Replaced(inp, out) => FundingIO::Replaced(inp, f(out)),
            FundingIO::Added(inp, out) => FundingIO::Added(inp, f(out)),
            FundingIO::NotUsed(inp) => FundingIO::NotUsed(inp),
        }
    }
}

impl<T> FundingIO<T, T> {
    pub fn into_effects(self) -> (Option<T>, Vec<FundingEff<T>>) {
        match self {
            FundingIO::Replaced(consumed, produced) => (
                None,
                vec![FundingEff::Consumed(consumed), FundingEff::Produced(produced)],
            ),
            FundingIO::Added(unused, produced) => (Some(unused), vec![FundingEff::Produced(produced)]),
            FundingIO::NotUsed(unused) => (Some(unused), vec![]),
        }
    }
}
