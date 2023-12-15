#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub enum TimeBounds<T> {
    /// X <= T
    Until(T),
    /// X >= T
    After(T),
    /// T <= X <= T
    Within(T, T),
    None,
}

impl<T> TimeBounds<T>
where
    T: Ord + Copy,
{
    pub fn contain(&self, time_slot: &T) -> bool {
        match self {
            TimeBounds::Until(t) => time_slot <= t,
            TimeBounds::After(t) => t >= time_slot,
            TimeBounds::Within(t0, t1) => t0 >= time_slot && time_slot <= t1,
            TimeBounds::None => true,
        }
    }
    pub fn lower_bound(&self) -> Option<T> {
        match self {
            TimeBounds::After(t) => Some(*t),
            TimeBounds::Within(t0, _) => Some(*t0),
            TimeBounds::Until(_) | TimeBounds::None => None,
        }
    }
}
