use serde::{Deserialize, Serialize};
use std::fmt::{Display, Formatter};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Ior<O1, O2> {
    Left(O1),
    Right(O2),
    Both(O1, O2),
}

impl<O1, O2> Display for Ior<O1, O2> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Ior::Left(_) => f.write_str("Ior::Left(_)"),
            Ior::Right(_) => f.write_str("Ior::Right(_)"),
            Ior::Both(_, _) => f.write_str("Ior::Both(_, _)"),
        }
    }
}

impl<O1, O2> Ior<O1, O2> {
    pub fn swap(self) -> Ior<O2, O1> {
        match self {
            Ior::Left(o1) => Ior::Right(o1),
            Ior::Right(o2) => Ior::Left(o2),
            Ior::Both(o1, o2) => Ior::Both(o2, o1),
        }
    }

    pub fn bimap<A, B, F1, F2>(self, lhf: F1, rhf: F2) -> Ior<A, B>
    where
        F1: FnOnce(O1) -> A,
        F2: FnOnce(O2) -> B,
    {
        match self {
            Ior::Left(lh) => Ior::Left(lhf(lh)),
            Ior::Right(rh) => Ior::Right(rhf(rh)),
            Ior::Both(lh, rh) => Ior::Both(lhf(lh), rhf(rh)),
        }
    }
}

impl<O1, O2> TryFrom<(Option<O1>, Option<O2>)> for Ior<O1, O2> {
    type Error = ();
    fn try_from(pair: (Option<O1>, Option<O2>)) -> Result<Self, Self::Error> {
        match pair {
            (Some(l), Some(r)) => Ok(Self::Both(l, r)),
            (Some(l), None) => Ok(Self::Left(l)),
            (None, Some(r)) => Ok(Self::Right(r)),
            _ => Err(()),
        }
    }
}
