pub enum ExecutionEffect<T, K> {
    Updated(T),
    Eliminated(K),
}
