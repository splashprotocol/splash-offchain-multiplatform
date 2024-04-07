pub trait Monoid {
    fn empty() -> Self;
    fn combine(self, other: Self) -> Self;
}

impl Monoid for u64 {
    fn empty() -> Self {
        0
    }
    fn combine(self, other: Self) -> Self {
        self + other
    }
}
