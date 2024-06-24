use crate::semigroup::Semigroup;

pub trait Monoid: Semigroup {
    fn empty() -> Self;
}

impl Semigroup for u64 {
    fn combine(self, other: Self) -> Self {
        self + other
    }
}

impl Monoid for u64 {
    fn empty() -> Self {
        0
    }
}
