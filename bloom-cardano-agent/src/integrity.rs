use algebra_core::semigroup::Semigroup;
use std::fmt::{Display, Formatter};

pub trait CheckIntegrity {
    fn check_integrity(&self) -> IntegrityViolations;
}

pub struct IntegrityViolations(pub Vec<String>);

impl Display for IntegrityViolations {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str("Integrity violations: ")?;
        for v in &self.0 {
            f.write_str(v.as_str())?;
            f.write_str(", ")?
        }
        Ok(())
    }
}

impl Semigroup for IntegrityViolations {
    fn combine(self, other: Self) -> Self {
        Self(self.0.into_iter().chain(other.0.into_iter()).collect())
    }
}

impl IntegrityViolations {
    pub fn empty() -> Self {
        IntegrityViolations(Vec::new())
    }
    pub fn one(violation: String) -> Self {
        IntegrityViolations(vec![violation])
    }
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}
