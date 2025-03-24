use crate::plutus_data::{ConstrPlutusDataExtension, PlutusDataExtension};
use crate::{AssetClass, AssetName, Token};
use cml_chain::plutus::PlutusData;
use cml_chain::PolicyId;
use cml_core::serialization::RawBytesEncoding;
use num_rational::Ratio;

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
            0 => Some(Some(T::try_from_pd(cpd.take_field(0)?)?)),
            1 => Some(None),
            _ => None,
        }
    }
}

impl<T> TryFromPData for Vec<T>
where
    T: TryFromPData,
{
    fn try_from_pd(data: PlutusData) -> Option<Self> {
        data.into_vec().and_then(|pd_vec| {
            pd_vec.into_iter().try_fold(vec![], |mut acc, raw_pd| {
                T::try_from_pd(raw_pd).map(|parsed_value| {
                    acc.push(parsed_value);
                    acc
                })
            })
        })
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

impl TryFromPData for PolicyId {
    fn try_from_pd(data: PlutusData) -> Option<Self> {
        Some(PolicyId::from(<[u8; 28]>::try_from(data.into_bytes()?).ok()?))
    }
}
