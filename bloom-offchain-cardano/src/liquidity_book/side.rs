use std::ops::Not;

#[derive(Debug, Copy, Clone)]
pub enum SideMarker {
    Bid,
    Ask,
}

impl Not for SideMarker {
    type Output = SideMarker;
    fn not(self) -> Self::Output {
        match self {
            SideMarker::Bid => SideMarker::Ask,
            SideMarker::Ask => SideMarker::Bid,
        }
    }
}

#[derive(Debug, Copy, Clone)]
pub enum Side<T> {
    Bid(T),
    Ask(T),
}

impl<T> Side<T> {
    pub fn any(&self) -> &T {
        match self {
            Side::Bid(t) => t,
            Side::Ask(t) => t,
        }
    }
    pub fn marker(&self) -> SideMarker {
        match self {
            Side::Bid(_) => SideMarker::Bid,
            Side::Ask(_) => SideMarker::Ask,
        }
    }
    pub fn map<R, F>(self, f: F) -> Side<R>
    where
        F: FnOnce(T) -> R,
    {
        match self {
            Side::Bid(t) => Side::Bid(f(t)),
            Side::Ask(t) => Side::Ask(f(t)),
        }
    }
}
