use std::fmt;
use std::fmt::Formatter;
use std::marker::PhantomData;

use crate::combinators::EitherOrBoth;
use serde::__private::de::missing_field;
use serde::ser::SerializeStruct;
use serde::{de, Deserialize, Deserializer, Serialize, Serializer};

use crate::data::OnChainEntity;

/// A unique, persistent, self-reproducible, on-chiain entity.
#[derive(Debug, Clone)]
pub struct Traced<TEntity: OnChainEntity> {
    pub state: TEntity,
    pub prev_state_id: Option<TEntity::TStateId>,
}

impl<TEntity: OnChainEntity> Serialize for Traced<TEntity>
where
    TEntity: Serialize,
    <TEntity as OnChainEntity>::TStateId: Serialize,
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

impl<'de, TEntity: OnChainEntity> Deserialize<'de> for Traced<TEntity>
where
    TEntity: Deserialize<'de>,
    <TEntity as OnChainEntity>::TStateId: Deserialize<'de>,
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
        struct Visitor<'de, TEntity: OnChainEntity>
        where
            TEntity: Deserialize<'de>,
        {
            marker: PhantomData<Traced<TEntity>>,
            lifetime: PhantomData<&'de ()>,
        }
        impl<'de, TEntity: OnChainEntity> de::Visitor<'de> for Visitor<'de, TEntity>
        where
            TEntity: Deserialize<'de>,
            <TEntity as OnChainEntity>::TStateId: Deserialize<'de>,
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
                let field1 = match match de::SeqAccess::next_element::<Option<TEntity::TStateId>>(&mut seq) {
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
                let mut field1: Option<Option<TEntity::TStateId>> = None;
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
                                match de::MapAccess::next_value::<Option<TEntity::TStateId>>(&mut map) {
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

/// Entity contexts:

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

impl<T: OnChainEntity> OnChainEntity for Predicted<T> {
    type TEntityId = T::TEntityId;
    type TStateId = T::TStateId;

    fn get_self_ref(&self) -> Self::TEntityId {
        self.0.get_self_ref()
    }

    fn get_self_state_ref(&self) -> Self::TStateId {
        self.0.get_self_state_ref()
    }
}

/// State `T` is confirmed to be included into blockchain.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Confirmed<T>(pub T);

/// State `T` was observed in mempool.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Unconfirmed<T>(pub T);

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
    Transition(EitherOrBoth<T, T>),
    /// State transition rollback (left: rolled back state, right: revived state).
    TransitionRollback(EitherOrBoth<T, T>),
}
