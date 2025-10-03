use crate::data::ior::Ior;
use crate::domain::{EntitySnapshot, SeqState, Stable};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::fmt::{Display, Formatter};

/// A unique, persistent, self-reproducible, on-chiain entity.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Traced<TEntity: EntitySnapshot> {
    pub state: TEntity,
    pub prev_state_id: Option<TEntity::Version>,
}

impl<TEntity: EntitySnapshot> Traced<TEntity> {
    pub fn new(state: TEntity, prev_state_id: Option<TEntity::Version>) -> Self {
        Self { state, prev_state_id }
    }
}

/// Any possible modality of `T`.
#[derive(Clone, Serialize, Deserialize)]
pub enum AnyMod<T: EntitySnapshot> {
    Confirmed(Traced<Confirmed<T>>),
    Predicted(Traced<Predicted<T>>),
}

impl<T: EntitySnapshot> AnyMod<T> {
    pub fn as_erased(&self) -> &T {
        match self {
            AnyMod::Confirmed(Traced {
                state: Confirmed(t), ..
            }) => t,
            AnyMod::Predicted(Traced {
                state: Predicted(t), ..
            }) => t,
        }
    }
    pub fn erased(self) -> T {
        match self {
            AnyMod::Confirmed(Traced {
                state: Confirmed(t), ..
            }) => t,
            AnyMod::Predicted(Traced {
                state: Predicted(t), ..
            }) => t,
        }
    }
}

/// Channel from which [T] was obtained.
#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
pub enum Channel<T, LedgerCx> {
    Ledger(Confirmed<T>, LedgerCx),
    Mempool(Unconfirmed<T>),
    LocalTxSubmit(Predicted<T>),
}

impl<T, Meta> Channel<T, Meta> {
    pub fn ledger(t: T, meta: Meta) -> Self {
        Self::Ledger(Confirmed(t), meta)
    }

    pub fn mempool(t: T) -> Self {
        Self::Mempool(Unconfirmed(t))
    }

    pub fn local_tx_submit(t: T) -> Self {
        Self::LocalTxSubmit(Predicted(t))
    }

    pub fn erased(&self) -> &T {
        match self {
            Channel::Ledger(Confirmed(t), _) => t,
            Channel::Mempool(Unconfirmed(t)) => t,
            Channel::LocalTxSubmit(Predicted(t)) => t,
        }
    }

    pub fn map<B, F>(self, f: F) -> Channel<B, Meta>
    where
        F: FnOnce(T) -> B,
    {
        match self {
            Channel::Ledger(Confirmed(x), m) => Channel::Ledger(Confirmed(f(x)), m),
            Channel::Mempool(Unconfirmed(x)) => Channel::Mempool(Unconfirmed(f(x))),
            Channel::LocalTxSubmit(Predicted(x)) => Channel::LocalTxSubmit(Predicted(f(x))),
        }
    }
}

impl<T: Stable, LedgerCx> Stable for Channel<T, LedgerCx> {
    type StableId = T::StableId;

    fn stable_id(&self) -> Self::StableId {
        self.erased().stable_id()
    }

    fn is_quasi_permanent(&self) -> bool {
        self.erased().is_quasi_permanent()
    }
}

impl<T: SeqState, LedgerCx> SeqState for Channel<T, LedgerCx> {
    fn is_initial(&self) -> bool {
        self.erased().is_initial()
    }
}

/// State `T` is confirmed to be included into blockchain.
#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct Confirmed<T>(pub T);

impl<T: Stable> Stable for Confirmed<T> {
    type StableId = T::StableId;

    fn stable_id(&self) -> Self::StableId {
        self.0.stable_id()
    }
    fn is_quasi_permanent(&self) -> bool {
        self.0.is_quasi_permanent()
    }
}

impl<T: SeqState> SeqState for Confirmed<T> {
    fn is_initial(&self) -> bool {
        self.0.is_initial()
    }
}

impl<T: EntitySnapshot> EntitySnapshot for Confirmed<T> {
    type Version = T::Version;

    fn version(&self) -> Self::Version {
        self.0.version()
    }
}

/// State `T` was observed in mempool.
#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct Unconfirmed<T>(pub T);

impl<T: Stable> Stable for Unconfirmed<T> {
    type StableId = T::StableId;

    fn stable_id(&self) -> Self::StableId {
        self.0.stable_id()
    }
    fn is_quasi_permanent(&self) -> bool {
        self.0.is_quasi_permanent()
    }
}

impl<T: SeqState> SeqState for Unconfirmed<T> {
    fn is_initial(&self) -> bool {
        self.0.is_initial()
    }
}

impl<T: EntitySnapshot> EntitySnapshot for Unconfirmed<T> {
    type Version = T::Version;

    fn version(&self) -> Self::Version {
        self.0.version()
    }
}

/// State `T` is predicted, but not confirmed to be included into blockchain or mempool yet.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Predicted<T>(pub T);

impl<T> Predicted<T> {
    pub fn map<U, F>(self, f: F) -> Predicted<U>
    where
        F: FnOnce(T) -> U,
    {
        Predicted(f(self.0))
    }
}

impl<T: Stable> Stable for Predicted<T> {
    type StableId = T::StableId;

    fn stable_id(&self) -> Self::StableId {
        self.0.stable_id()
    }
    fn is_quasi_permanent(&self) -> bool {
        self.0.is_quasi_permanent()
    }
}

impl<T: SeqState> SeqState for Predicted<T> {
    fn is_initial(&self) -> bool {
        self.0.is_initial()
    }
}

impl<T: EntitySnapshot> EntitySnapshot for Predicted<T> {
    type Version = T::Version;

    fn version(&self) -> Self::Version {
        self.0.version()
    }
}

#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
pub enum Transition<T> {
    /// State transition (left: old state, right: new state).
    Forward(Ior<T, T>),
    /// State transition rollback (left: rolled back state, right: revived state).
    Backward(Ior<T, T>),
}

impl<T> Transition<T> {
    pub fn is_rollback(&self) -> bool {
        matches!(self, Transition::Backward(_))
    }
}

impl<T: Stable> Stable for Transition<T> {
    type StableId = T::StableId;

    fn stable_id(&self) -> Self::StableId {
        match self {
            Transition::Forward(ior) | Transition::Backward(ior) => ior.stable_id(),
        }
    }
    fn is_quasi_permanent(&self) -> bool {
        match self {
            Transition::Forward(ior) | Transition::Backward(ior) => ior.is_quasi_permanent(),
        }
    }
}

impl<T> Display for Transition<T>
where
    T: Display,
{
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Transition::Forward(tr) => f.write_str(format!("Forward({})", tr).as_str()),
            Transition::Backward(tr) => f.write_str(format!("Backward({})", tr).as_str()),
        }
    }
}

impl<T> Transition<T> {
    pub fn map<B, F>(self, f: F) -> Transition<B>
    where
        F: Fn(T) -> B,
    {
        match self {
            Transition::Forward(ior) => Transition::Forward(ior.bimap(&f, &f)),
            Transition::Backward(ior) => Transition::Backward(ior.bimap(&f, &f)),
        }
    }

    pub fn inspect<F>(&self, f: F) -> bool
    where
        F: Fn(&T) -> bool + Clone,
    {
        match self {
            Transition::Forward(ior) => ior.inspect(f),
            Transition::Backward(ior) => ior.inspect(f),
        }
    }
}
