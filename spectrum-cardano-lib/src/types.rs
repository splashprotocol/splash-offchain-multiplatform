use cml_chain::plutus::PlutusData;
use num_rational::Ratio;

use crate::plutus_data::{ConstrPlutusDataExtension, PlutusDataExtension};

/// Tries to parse `Self` from `PlutusData`.
pub trait TryFromPData: Sized {
    fn try_from_pd(data: PlutusData) -> Option<Self>;
}

impl TryFromPData for bool {
    fn try_from_pd(data: PlutusData) -> Option<Self> {
        let cpd = data.into_constr_pd()?;
        match cpd.alternative {
            0 => Some(false),
            1 => Some(true),
            _ => None,
        }
    }
}

impl<T> TryFromPData for Option<T>
where
    T: TryFromPData,
{
    fn try_from_pd(data: PlutusData) -> Option<Self> {
        let mut cpd = data.into_constr_pd()?;
        match cpd.alternative {
            0 => Some(Some(T::try_from_pd(cpd.take_field(0)?))?),
            1 => Some(None),
            _ => None,
        }
    }
}

impl TryFromPData for Ratio<u128> {
    fn try_from_pd(data: PlutusData) -> Option<Self> {
        let mut cpd = data.into_constr_pd()?;
        let denom = cpd.take_field(1)?.into_u128()?;
        if denom != 0 {
            Some(Ratio::new_raw(cpd.take_field(0)?.into_u128()?, denom))
        } else {
            None
        }
    }
}
