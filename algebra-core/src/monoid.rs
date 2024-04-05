pub trait Monoid {
    fn identity() -> Self;
    fn combine(self, other: Self) -> Self;
}

impl Monoid for u64 {
    fn identity() -> Self {
        0
    }
    fn combine(self, other: Self) -> Self {
        self + other
    }
}