use cml_chain::plutus::PlutusData;

use spectrum_cardano_lib::plutus_data::ConstrPlutusDataExtension;
use spectrum_cardano_lib::plutus_data::PlutusDataExtension;
use spectrum_cardano_lib::types::TryFromPData;
use spectrum_cardano_lib::{TaggedAmount, TaggedAssetClass};

use crate::data::order::PoolNft;
use crate::data::pool::{Lq, Rx, Ry};

pub struct FeeSwitchPoolConfig {
    pub pool_nft: TaggedAssetClass<PoolNft>,
    pub asset_x: TaggedAssetClass<Rx>,
    pub asset_y: TaggedAssetClass<Ry>,
    pub asset_lq: TaggedAssetClass<Lq>,
    pub lp_fee_num: u64,
    pub treasury_fee_num: u64,
    pub treasury_x: u64,
    pub treasury_y: u64,
    pub lq_lower_bound: TaggedAmount<Lq>,
}

impl TryFromPData for FeeSwitchPoolConfig {
    fn try_from_pd(data: PlutusData) -> Option<Self> {
        let mut cpd = data.into_constr_pd()?;
        Some(Self {
            pool_nft: TaggedAssetClass::try_from_pd(cpd.take_field(0)?)?,
            asset_x: TaggedAssetClass::try_from_pd(cpd.take_field(1)?)?,
            asset_y: TaggedAssetClass::try_from_pd(cpd.take_field(2)?)?,
            asset_lq: TaggedAssetClass::try_from_pd(cpd.take_field(3)?)?,
            lp_fee_num: cpd.take_field(4)?.into_u64()?,
            treasury_fee_num: cpd.take_field(5)?.into_u64()?,
            treasury_x: cpd.take_field(6)?.into_u64()?,
            treasury_y: cpd.take_field(7)?.into_u64()?,
            lq_lower_bound: TaggedAmount::new(cpd.take_field(9).and_then(|pd| pd.into_u64()).unwrap_or(0)),
        })
    }
}

mod tests {

    use crate::data::fee_switch_pool::FeeSwitchPoolConfig;
    use cml_chain::plutus::PlutusData;
    use cml_chain::Deserialize;
    use spectrum_cardano_lib::types::TryFromPData;

    const DATUM_SAMPLE: &str =
        "d8799fd8799f581c6aaa652b39f5723afc85bba38401a4cbfd5b2f7aa3771504257ac8a74d74657374425f4144415f4e4654ffd8799f4040ffd8799f581c4b3459fd18a1dbabe207cd19c9951a9fac9f5c0f9c384e3d97efba26457465737442ffd8799f581c635f44ae5df86be9e80fd0c57a5ec699a146d9d9034516ffd72febef4c74657374425f4144415f4c51ff19270b010000801b00000002540be400581c2618e94cdb06792f05ae9b1ec78b0231f4b7f4215b1b4cf52e6342deff";

    #[test]
    fn parse_fee_switch_datum_mainnet() {
        let pd = PlutusData::from_cbor_bytes(&*hex::decode(DATUM_SAMPLE).unwrap()).unwrap();
        let maybe_conf = FeeSwitchPoolConfig::try_from_pd(pd);
        assert!(maybe_conf.is_some())
    }
}
