use cml_chain::plutus::PlutusData;

/// Tries to parse `Self` from `PlutusData`.
pub trait TryFromPData: Sized {
    fn try_from_pd(data: PlutusData) -> Option<Self>;
}
