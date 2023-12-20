use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Ior<O1, O2> {
    Left(O1),
    Right(O2),
    Both(O1, O2),
}

impl<O1, O2> Ior<O1, O2> {
    pub fn swap(self) -> Ior<O2, O1> {
        match self {
            Ior::Left(o1) => Ior::Right(o1),
            Ior::Right(o2) => Ior::Left(o2),
            Ior::Both(o1, o2) => Ior::Both(o2, o1),
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
