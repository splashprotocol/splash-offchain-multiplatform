pub enum ExecutionEffect<T, K> {
    Updated(T),
    Eliminated(K),
}

impl<T, K> ExecutionEffect<T, K> {
    pub fn map_eliminated<K2, F>(self, f: F) -> ExecutionEffect<T, K2>
    where
        F: FnOnce(K) -> K2,
    {
        match self {
            ExecutionEffect::Eliminated(e) => ExecutionEffect::Eliminated(f(e)),
            ExecutionEffect::Updated(u) => ExecutionEffect::Updated(u),
        }
    }
}
