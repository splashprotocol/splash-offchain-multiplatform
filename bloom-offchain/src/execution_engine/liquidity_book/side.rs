use std::ops::Not;

use derive_more::Display;

/// Side marker.
#[derive(Debug, Display, Copy, Clone, PartialEq, Eq, Hash)]
pub enum SideM {
    Bid,
    Ask,
}

impl SideM {
    pub fn wrap<T>(self, value: T) -> Side<T> {
        match self {
            SideM::Bid => Side::Bid(value),
            SideM::Ask => Side::Ask(value),
        }
    }
}

impl Not for SideM {
    type Output = SideM;
    fn not(self) -> Self::Output {
        match self {
            SideM::Bid => SideM::Ask,
            SideM::Ask => SideM::Bid,
        }
    }
}

#[derive(Debug, Copy, Clone, Eq, PartialEq, Hash)]
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
    pub fn unwrap(self) -> T {
        match self {
            Side::Bid(t) => t,
            Side::Ask(t) => t,
        }
    }
    pub fn marker(&self) -> SideM {
        match self {
            Side::Bid(_) => SideM::Bid,
            Side::Ask(_) => SideM::Ask,
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
