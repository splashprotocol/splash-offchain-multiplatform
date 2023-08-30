use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum EitherOrBoth<O1, O2> {
    Left(O1),
    Right(O2),
    Both(O1, O2),
}

impl<O1, O2> EitherOrBoth<O1, O2> {
    pub fn swap(self) -> EitherOrBoth<O2, O1> {
        match self {
            EitherOrBoth::Left(o1) => EitherOrBoth::Right(o1),
            EitherOrBoth::Right(o2) => EitherOrBoth::Left(o2),
            EitherOrBoth::Both(o1, o2) => EitherOrBoth::Both(o2, o1),
        }
    }
}

impl<O1, O2> TryFrom<(Option<O1>, Option<O2>)> for EitherOrBoth<O1, O2> {
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
