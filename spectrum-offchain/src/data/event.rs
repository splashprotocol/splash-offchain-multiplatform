use std::fmt;
use std::fmt::Formatter;
use std::marker::PhantomData;

use serde::__private::de::missing_field;
use serde::ser::SerializeStruct;
use serde::{de, Deserialize, Deserializer, Serialize, Serializer};

use crate::combinators::Ior;
use crate::data::{EntitySnapshot, Stable};

/// A unique, persistent, self-reproducible, on-chiain entity.
#[derive(Debug, Clone)]
pub struct Traced<TEntity: EntitySnapshot> {
    pub state: TEntity,
    pub prev_state_id: Option<TEntity::Version>,
}

impl<TEntity: EntitySnapshot> Traced<TEntity> {
    pub fn new(state: TEntity, prev_state_id: Option<TEntity::Version>) -> Self {
        Self { state, prev_state_id }
    }
}

impl<TEntity: EntitySnapshot> Serialize for Traced<TEntity>
where
    TEntity: Serialize,
    <TEntity as EntitySnapshot>::Version: Serialize,
{
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut serde_state = match serializer.serialize_struct("Traced", false as usize + 1 + 1) {
            Ok(val) => val,
            Err(err) => {
                return Err(err);
            }
        };
        match serde_state.serialize_field("state", &self.state) {
            Ok(val) => val,
            Err(err) => {
                return Err(err);
            }
        };
        match serde_state.serialize_field("prev_state_id", &self.prev_state_id) {
            Ok(val) => val,
            Err(err) => {
                return Err(err);
            }
        };
        serde_state.end()
    }
}

impl<'de, TEntity: EntitySnapshot> Deserialize<'de> for Traced<TEntity>
where
    TEntity: Deserialize<'de>,
    <TEntity as EntitySnapshot>::Version: Deserialize<'de>,
{
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        enum Field {
            State,
            PrevStateId,
            Ignore,
        }
        struct FieldVisitor;
        impl<'de> de::Visitor<'de> for FieldVisitor {
            type Value = Field;
            fn expecting(&self, formatter: &mut Formatter) -> fmt::Result {
                Formatter::write_str(formatter, "field identifier")
            }
            fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                match value {
                    0u64 => Ok(Field::State),
                    1u64 => Ok(Field::PrevStateId),
                    _ => Ok(Field::Ignore),
                }
            }
            fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                match value {
                    "state" => Ok(Field::State),
                    "prev_state_id" => Ok(Field::PrevStateId),
                    _ => Ok(Field::Ignore),
                }
            }
            fn visit_bytes<E>(self, value: &[u8]) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                match value {
                    b"state" => Ok(Field::State),
                    b"prev_state_id" => Ok(Field::PrevStateId),
                    _ => Ok(Field::Ignore),
                }
            }
        }
        impl<'de> Deserialize<'de> for Field {
            #[inline]
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: Deserializer<'de>,
            {
                Deserializer::deserialize_identifier(deserializer, FieldVisitor)
            }
        }
        struct Visitor<'de, TEntity: EntitySnapshot>
        where
            TEntity: Deserialize<'de>,
        {
            marker: PhantomData<Traced<TEntity>>,
            lifetime: PhantomData<&'de ()>,
        }
        impl<'de, TEntity: EntitySnapshot> de::Visitor<'de> for Visitor<'de, TEntity>
        where
            TEntity: Deserialize<'de>,
            <TEntity as EntitySnapshot>::Version: Deserialize<'de>,
        {
            type Value = Traced<TEntity>;
            fn expecting(&self, formatter: &mut Formatter) -> fmt::Result {
                Formatter::write_str(formatter, "struct Traced")
            }
            #[inline]
            fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
            where
                A: de::SeqAccess<'de>,
            {
                let field0 = match match de::SeqAccess::next_element::<TEntity>(&mut seq) {
                    Ok(val) => val,
                    Err(err) => {
                        return Err(err);
                    }
                } {
                    Some(value) => value,
                    None => {
                        return Err(de::Error::invalid_length(
                            0usize,
                            &"struct Traced with 2 elements",
                        ));
                    }
                };
                let field1 = match match de::SeqAccess::next_element::<Option<TEntity::Version>>(&mut seq) {
                    Ok(val) => val,
                    Err(err) => {
                        return Err(err);
                    }
                } {
                    Some(value) => value,
                    None => {
                        return Err(de::Error::invalid_length(
                            1usize,
                            &"struct Traced with 2 elements",
                        ));
                    }
                };
                Ok(Traced {
                    state: field0,
                    prev_state_id: field1,
                })
            }
            #[inline]
            fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
            where
                A: de::MapAccess<'de>,
            {
                let mut field0: Option<TEntity> = None;
                let mut field1: Option<Option<TEntity::Version>> = None;
                while let Some(key) = match de::MapAccess::next_key::<Field>(&mut map) {
                    Ok(val) => val,
                    Err(err) => {
                        return Err(err);
                    }
                } {
                    match key {
                        Field::State => {
                            if Option::is_some(&field0) {
                                return Err(<A::Error as de::Error>::duplicate_field("state"));
                            }
                            field0 = Some(match de::MapAccess::next_value::<TEntity>(&mut map) {
                                Ok(val) => val,
                                Err(err) => {
                                    return Err(err);
                                }
                            });
                        }
                        Field::PrevStateId => {
                            if Option::is_some(&field1) {
                                return Err(<A::Error as de::Error>::duplicate_field("prev_state_id"));
                            }
                            field1 = Some(
                                match de::MapAccess::next_value::<Option<TEntity::Version>>(&mut map) {
                                    Ok(val) => val,
                                    Err(err) => {
                                        return Err(err);
                                    }
                                },
                            );
                        }
                        _ => {
                            let _ = match de::MapAccess::next_value::<de::IgnoredAny>(&mut map) {
                                Ok(val) => val,
                                Err(err) => {
                                    return Err(err);
                                }
                            };
                        }
                    }
                }
                let field0 = match field0 {
                    Some(field0) => field0,
                    None => match missing_field("state") {
                        Ok(val) => val,
                        Err(err) => {
                            return Err(err);
                        }
                    },
                };
                let field1 = match field1 {
                    Some(field1) => field1,
                    None => match missing_field("prev_state_id") {
                        Ok(val) => val,
                        Err(err) => {
                            return Err(err);
                        }
                    },
                };
                Ok(Traced {
                    state: field0,
                    prev_state_id: field1,
                })
            }
        }
        const FIELDS: &[&str] = &["state", "prev_state_id"];
        Deserializer::deserialize_struct(
            deserializer,
            "Traced",
            FIELDS,
            Visitor {
                marker: PhantomData::<Traced<TEntity>>,
                lifetime: PhantomData,
            },
        )
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
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Channel<T> {
    Ledger(Confirmed<T>),
    Mempool(Unconfirmed<T>),
    LocalTxSubmit(Predicted<T>),
}

impl<T> Channel<T> {
    pub fn ledger(t: T) -> Self {
        Self::Ledger(Confirmed(t))
    }

    pub fn mempool(t: T) -> Self {
        Self::Mempool(Unconfirmed(t))
    }

    pub fn local_tx_submit(t: T) -> Self {
        Self::LocalTxSubmit(Predicted(t))
    }

    pub fn erased(&self) -> &T {
        match self {
            Channel::Ledger(Confirmed(t)) => t,
            Channel::Mempool(Unconfirmed(t)) => t,
            Channel::LocalTxSubmit(Predicted(t)) => t,
        }
    }
    pub fn map<B, F>(self, f: F) -> Channel<B>
    where
        F: FnOnce(T) -> B,
    {
        match self {
            Channel::Ledger(Confirmed(x)) => Channel::Ledger(Confirmed(f(x))),
            Channel::Mempool(Unconfirmed(x)) => Channel::Mempool(Unconfirmed(f(x))),
            Channel::LocalTxSubmit(Predicted(x)) => Channel::LocalTxSubmit(Predicted(f(x))),
        }
    }
}

/// State `T` is confirmed to be included into blockchain.
#[derive(Debug, Clone, Serialize, Deserialize)]
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

impl<T: EntitySnapshot> EntitySnapshot for Confirmed<T> {
    type Version = T::Version;

    fn version(&self) -> Self::Version {
        self.0.version()
    }
}

/// State `T` was observed in mempool.
#[derive(Debug, Clone, Serialize, Deserialize)]
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

impl<T: EntitySnapshot> EntitySnapshot for Unconfirmed<T> {
    type Version = T::Version;

    fn version(&self) -> Self::Version {
        self.0.version()
    }
}

/// State `T` is predicted, but not confirmed to be included into blockchain or mempool yet.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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

impl<T: EntitySnapshot> EntitySnapshot for Predicted<T> {
    type Version = T::Version;

    fn version(&self) -> Self::Version {
        self.0.version()
    }
}

/// How states apply to the sequence of states of an entity:

/// State is applied on top of previous states.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Upgrade<T>(pub T);

/// State is discarded and should be eliminated from the sequence of upgrades.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpgradeRollback<T>(pub T);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum StateUpdate<T> {
    /// State transition (left: old state, right: new state).
    Transition(Ior<T, T>),
    /// State transition rollback (left: rolled back state, right: revived state).
    TransitionRollback(Ior<T, T>),
}

impl<T> StateUpdate<T> {
    pub fn map<B, F>(self, f: F) -> StateUpdate<B>
    where
        F: Fn(T) -> B,
    {
        match self {
            StateUpdate::Transition(ior) => StateUpdate::Transition(ior.bimap(&f, &f)),
            StateUpdate::TransitionRollback(ior) => StateUpdate::TransitionRollback(ior.bimap(&f, &f)),
        }
    }
}
