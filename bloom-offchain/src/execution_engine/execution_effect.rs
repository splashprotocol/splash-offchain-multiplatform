pub enum ExecutionEff<T, K> {
    Updated(T),
    Eliminated(K),
}

impl<T, K> ExecutionEff<T, K> {
    pub fn map_eliminated<K2, F>(self, f: F) -> ExecutionEff<T, K2>
    where
        F: FnOnce(K) -> K2,
    {
        match self {
            ExecutionEff::Eliminated(e) => ExecutionEff::Eliminated(f(e)),
            ExecutionEff::Updated(u) => ExecutionEff::Updated(u),
        }
    }
}
