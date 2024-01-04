use cml_chain::plutus::PlutusData;
use num_rational::Ratio;

use crate::plutus_data::{ConstrPlutusDataExtension, PlutusDataExtension};

/// Tries to parse `Self` from `PlutusData`.
pub trait TryFromPData: Sized {
    fn try_from_pd(data: PlutusData) -> Option<Self>;
}

impl TryFromPData for Ratio<u128> {
    fn try_from_pd(data: PlutusData) -> Option<Self> {
        let mut cpd = data.into_constr_pd()?;
        Some(Ratio::new(
            cpd.take_field(0)?.into_u64()? as u128,
            cpd.take_field(1)?.into_u64()? as u128,
        ))
    }
}
