pub trait Semigroup {
    fn combine(self, other: Self) -> Self;
}
