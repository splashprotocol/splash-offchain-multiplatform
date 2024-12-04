use derive_more::Display;
use serde::{Deserialize, Serialize};
use std::fmt::Formatter;
use std::ops::Not;

/// Side marker.
#[derive(Debug, Display, Copy, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Side {
    Bid,
    Ask,
}

impl Side {
    pub fn wrap<T>(self, value: T) -> OnSide<T> {
        match self {
            Side::Bid => OnSide::Bid(value),
            Side::Ask => OnSide::Ask(value),
        }
    }
}

impl Not for Side {
    type Output = Side;
    fn not(self) -> Self::Output {
        match self {
            Side::Bid => Side::Ask,
            Side::Ask => Side::Bid,
        }
    }
}

#[derive(Debug, Copy, Clone, Eq, PartialEq, Hash)]
pub enum OnSide<T> {
    Bid(T),
    Ask(T),
}

impl<T: Display> Display for OnSide<T> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            OnSide::Bid(bid) => f.write_str(&*format!("Bid({})", bid)),
            OnSide::Ask(ask) => f.write_str(&*format!("Ask({})", ask)),
        }
    }
}

impl<T> OnSide<T> {
    pub fn any(&self) -> &T {
        match self {
            OnSide::Bid(t) => t,
            OnSide::Ask(t) => t,
        }
    }
    pub fn unwrap(self) -> T {
        match self {
            OnSide::Bid(t) => t,
            OnSide::Ask(t) => t,
        }
    }
    pub fn marker(&self) -> Side {
        match self {
            OnSide::Bid(_) => Side::Bid,
            OnSide::Ask(_) => Side::Ask,
        }
    }
    pub fn map<R, F>(self, f: F) -> OnSide<R>
    where
        F: FnOnce(T) -> R,
    {
        match self {
            OnSide::Bid(t) => OnSide::Bid(f(t)),
            OnSide::Ask(t) => OnSide::Ask(f(t)),
        }
    }
}
