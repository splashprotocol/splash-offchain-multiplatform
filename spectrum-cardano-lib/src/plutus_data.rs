use std::mem;

use cml_chain::plutus::{ConstrPlutusData, PlutusData};
use cml_chain::transaction::DatumOption;
use cml_chain::utils::BigInt;
use cml_core::serialization::LenEncoding;

/// Some on-chain entities may require a redeemer for a specific action.
pub trait RequiresRedeemer<Redeemer> {
    fn redeemer(self, action: Redeemer) -> PlutusData;
}

pub trait IntoPlutusData {
    fn into_pd(self) -> PlutusData;
}

impl IntoPlutusData for u64 {
    fn into_pd(self) -> PlutusData {
        PlutusData::Integer(BigInt::from(self))
    }
}

impl IntoPlutusData for ConstrPlutusData {
    fn into_pd(self) -> PlutusData {
        PlutusData::ConstrPlutusData(self)
    }
}

pub trait PlutusDataExtension {
    fn into_constr_pd(self) -> Option<ConstrPlutusData>;
    fn get_constr_pd_mut(&mut self) -> Option<&mut ConstrPlutusData>;
    fn into_bytes(self) -> Option<Vec<u8>>;
    fn into_u64(self) -> Option<u64>;
    fn into_u128(self) -> Option<u128>;
    fn into_vec_pd<T>(self, f: fn(PlutusData) -> Option<T>) -> Option<Vec<T>>;
    fn into_vec(self) -> Option<Vec<PlutusData>>;
}

impl PlutusDataExtension for PlutusData {
    fn into_constr_pd(self) -> Option<ConstrPlutusData> {
        match self {
            PlutusData::ConstrPlutusData(cpd) => Some(cpd),
            _ => None,
        }
    }

    fn get_constr_pd_mut(&mut self) -> Option<&mut ConstrPlutusData> {
        match self {
            PlutusData::ConstrPlutusData(cpd) => Some(cpd),
            _ => None,
        }
    }

    fn into_bytes(self) -> Option<Vec<u8>> {
        match self {
            PlutusData::Bytes { bytes, .. } => Some(bytes),
            _ => None,
        }
    }

    fn into_u64(self) -> Option<u64> {
        match self {
            PlutusData::Integer(big_int) => Some(big_int.as_u64()?),
            _ => None,
        }
    }

    fn into_u128(self) -> Option<u128> {
        match self {
            PlutusData::Integer(big_int) => Some(big_int.as_u128()?),
            _ => None,
        }
    }

    fn into_vec(self) -> Option<Vec<PlutusData>> {
        match self {
            PlutusData::List { list, .. } => Some(list),
            _ => None,
        }
    }

    fn into_vec_pd<T>(self, f: fn(PlutusData) -> Option<T>) -> Option<Vec<T>> {
        match self {
            PlutusData::List { list, .. } => Some(list.into_iter().flat_map(f).collect()),
            _ => None,
        }
    }
}

const DUMMY_PD: PlutusData = PlutusData::List {
    list: vec![],
    list_encoding: LenEncoding::Canonical,
};

pub trait ConstrPlutusDataExtension {
    /// Takes field `PlutusData` at the specified `index` replacing it with a dummy value.
    fn take_field(&mut self, index: usize) -> Option<PlutusData>;
    fn set_field(&mut self, index: usize, value: PlutusData);
    fn update_field<F>(&mut self, index: usize, f: F)
    where
        F: FnOnce(PlutusData) -> PlutusData;

    fn update_field_unsafe(&mut self, index: usize, new_value: PlutusData);
}

impl ConstrPlutusDataExtension for ConstrPlutusData {
    fn take_field(&mut self, index: usize) -> Option<PlutusData> {
        let mut pd = DUMMY_PD;
        mem::swap(&mut pd, self.fields.get_mut(index)?);
        Some(pd)
    }
    fn set_field(&mut self, index: usize, value: PlutusData) {
        self.fields.insert(index, value)
    }
    fn update_field<F>(&mut self, index: usize, f: F)
    where
        F: FnOnce(PlutusData) -> PlutusData,
    {
        if let Some(fld) = self.fields.get_mut(index) {
            let mut pd = DUMMY_PD;
            mem::swap(&mut pd, fld);
            let mut updated_pd = f(pd);
            mem::swap(&mut updated_pd, fld);
        }
    }

    fn update_field_unsafe(&mut self, index: usize, new_value: PlutusData) {
        if let Some(fld) = self.fields.get_mut(index) {
            let mut pd = new_value.clone();
            mem::swap(&mut pd, fld)
        }
    }
}

pub trait DatumExtension {
    fn into_pd(self) -> Option<PlutusData>;
}

impl DatumExtension for DatumOption {
    fn into_pd(self) -> Option<PlutusData> {
        match self {
            DatumOption::Datum { datum, .. } => Some(datum),
            DatumOption::Hash { .. } => None,
        }
    }
}
