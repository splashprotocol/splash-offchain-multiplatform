use crate::execution_engine::SourceId;

pub trait SourceDB<Src> {
    /// Query resolved source by [SourceId].
    /// Source is always available.
    fn get(&mut self, source_id: SourceId) -> Src;
}
