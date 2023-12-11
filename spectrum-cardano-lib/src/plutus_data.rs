use std::mem;

use cml_chain::plutus::{ConstrPlutusData, PlutusData};
use cml_chain::transaction::DatumOption;
use cml_core::serialization::LenEncoding;

/// Some on-chain entities may require a redeemer for a specific action.
pub trait RequiresRedeemer<Action> {
    fn redeemer(action: Action) -> PlutusData;
}

pub trait PlutusDataExtension {
    fn into_constr_pd(self) -> Option<ConstrPlutusData>;
    fn into_bytes(self) -> Option<Vec<u8>>;
    fn into_u64(self) -> Option<u64>;
}

impl PlutusDataExtension for PlutusData {
    fn into_constr_pd(self) -> Option<ConstrPlutusData> {
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
}

const DUMMY_PD: PlutusData = PlutusData::List {
    list: vec![],
    list_encoding: LenEncoding::Canonical,
};

pub trait ConstrPlutusDataExtension {
    /// Takes field `PlutusData` at the specified `index` replacing it with a dummy value.
    fn take_field(&mut self, index: usize) -> Option<PlutusData>;
}

impl ConstrPlutusDataExtension for ConstrPlutusData {
    fn take_field(&mut self, index: usize) -> Option<PlutusData> {
        let mut pd = DUMMY_PD;
        mem::swap(&mut pd, self.fields.get_mut(index)?);
        Some(pd)
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
