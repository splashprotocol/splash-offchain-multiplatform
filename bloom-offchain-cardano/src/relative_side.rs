use cml_chain::plutus::{ConstrPlutusData, PlutusData};
use derive_more::{From, Into};

use bloom_offchain::execution_engine::liquidity_book::side::Side;
use spectrum_cardano_lib::plutus_data::IntoPlutusData;

#[derive(Debug, Copy, Clone, PartialEq, Eq, Into, From)]
pub struct RelativeSide(Side);
impl RelativeSide {
    pub fn value(self) -> Side {
        self.0
    }
}

impl IntoPlutusData for RelativeSide {
    fn into_pd(self) -> PlutusData {
        let alt = match self.0 {
            Side::Bid => 1,
            Side::Ask => 0,
        };
        PlutusData::ConstrPlutusData(ConstrPlutusData::new(alt, vec![]))
    }
}
