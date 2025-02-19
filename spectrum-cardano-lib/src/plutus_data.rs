use std::mem;
use std::str::FromStr;

use cml_chain::plutus::utils::ConstrPlutusDataEncoding;
use cml_chain::plutus::{ConstrPlutusData, PlutusData};
use cml_chain::transaction::DatumOption;
use cml_chain::utils::BigInteger;
use cml_core::serialization::LenEncoding;
use cml_core::serialization::LenEncoding::{Canonical, Indefinite};
use num_rational::Ratio;
use primitive_types::U512;

pub trait IntoPlutusData {
    fn into_pd(self) -> PlutusData;
}

impl IntoPlutusData for u64 {
    fn into_pd(self) -> PlutusData {
        PlutusData::Integer(BigInteger::from(self))
    }
}

impl IntoPlutusData for i64 {
    fn into_pd(self) -> PlutusData {
        PlutusData::Integer(BigInteger::from(self))
    }
}

impl IntoPlutusData for u128 {
    fn into_pd(self) -> PlutusData {
        PlutusData::Integer(BigInteger::from(self))
    }
}

impl IntoPlutusData for Vec<u8> {
    fn into_pd(self) -> PlutusData {
        PlutusData::Bytes {
            bytes: self,
            bytes_encoding: Default::default(),
        }
    }
}

impl IntoPlutusData for U512 {
    fn into_pd(self) -> PlutusData {
        PlutusData::Integer(BigInteger::from_str(self.to_string().as_str()).unwrap())
    }
}

impl<const N: usize> IntoPlutusData for [u8; N] {
    fn into_pd(self) -> PlutusData {
        PlutusData::new_bytes(self.to_vec())
    }
}

impl IntoPlutusData for ConstrPlutusData {
    fn into_pd(self) -> PlutusData {
        PlutusData::ConstrPlutusData(self)
    }
}

impl<T: IntoPlutusData + Copy> IntoPlutusData for Ratio<T> {
    fn into_pd(self) -> PlutusData {
        PlutusData::ConstrPlutusData(ConstrPlutusData::new(
            0,
            vec![(*self.numer()).into_pd(), (*self.denom()).into_pd()],
        ))
    }
}

impl<T: IntoPlutusData + Clone> IntoPlutusData for Vec<T> {
    fn into_pd(self) -> PlutusData {
        PlutusData::List {
            list: self.iter().map(|to_pd| to_pd.clone().into_pd()).collect(),
            list_encoding: LenEncoding::Indefinite,
        }
    }
}

impl<T: IntoPlutusData + Clone> IntoPlutusData for Option<T> {
    fn into_pd(self) -> PlutusData {
        match self {
            None => PlutusData::new_constr_plutus_data(ConstrPlutusData {
                alternative: 1,
                fields: vec![],
                encodings: Some(ConstrPlutusDataEncoding {
                    len_encoding: Canonical,
                    tag_encoding: Some(cbor_event::Sz::One),
                    alternative_encoding: None,
                    fields_encoding: Indefinite,
                    prefer_compact: true,
                }),
            }),
            Some(to_pd) => PlutusData::new_constr_plutus_data(ConstrPlutusData {
                alternative: 0,
                fields: vec![to_pd.into_pd()],
                encodings: Some(ConstrPlutusDataEncoding {
                    len_encoding: Canonical,
                    tag_encoding: Some(cbor_event::Sz::One),
                    alternative_encoding: None,
                    fields_encoding: Indefinite,
                    prefer_compact: true,
                }),
            }),
        }
    }
}

pub trait PlutusDataExtension {
    fn into_constr_pd(self) -> Option<ConstrPlutusData>;
    fn get_constr_pd_mut(&mut self) -> Option<&mut ConstrPlutusData>;
    fn into_bytes(self) -> Option<Vec<u8>>;
    fn into_u64(self) -> Option<u64>;
    fn into_i128(self) -> Option<i128>;
    fn into_u128(self) -> Option<u128>;
    fn into_u512(self) -> Option<U512>;
    fn into_vec_pd<T>(self, f: fn(PlutusData) -> Option<T>) -> Option<Vec<T>>;
    fn into_vec(self) -> Option<Vec<PlutusData>>;
    fn into_pd_map(self) -> Option<Vec<(PlutusData, PlutusData)>>;
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

    fn into_i128(self) -> Option<i128> {
        match self {
            PlutusData::Integer(big_int) => Some(i128::from(&big_int.as_int()?)),
            _ => None,
        }
    }

    fn into_u128(self) -> Option<u128> {
        match self {
            PlutusData::Integer(big_int) => Some(big_int.as_u128()?),
            _ => None,
        }
    }

    fn into_u512(self) -> Option<U512> {
        match self {
            PlutusData::Integer(big_int) => U512::from_str_radix(big_int.to_string().as_str(), 10).ok(),
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

    fn into_pd_map(self) -> Option<Vec<(PlutusData, PlutusData)>> {
        match self {
            PlutusData::Map(m) => Some(m.entries),
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
        self.fields[index] = value;
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

/// Constructs a ConstrPlutusData instance which is bitwise-exact with how Aiken constructs such
/// values. This is essential if we want to check equality of serialised PlutusData values.
pub fn make_constr_pd_indefinite_arr(fields: Vec<PlutusData>) -> PlutusData {
    let enc = ConstrPlutusDataEncoding {
        len_encoding: LenEncoding::Indefinite,
        prefer_compact: true,
        tag_encoding: None,
        alternative_encoding: None,
        fields_encoding: LenEncoding::Indefinite,
    };
    PlutusData::new_constr_plutus_data(ConstrPlutusData {
        alternative: 0,
        fields,
        encodings: Some(enc),
    })
}
