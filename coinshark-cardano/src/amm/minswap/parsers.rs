use cml_multi_era::babbage::BabbageTransactionOutput;

use spectrum_cardano_lib::{AssetClass, AssetName, OutputRef, TaggedAmount, TaggedAssetClass};
use spectrum_cardano_lib::transaction::TransactionOutputExtension;

use crate::amm::{Platform, PoolId, PoolVersion};
use crate::amm::minswap::constants::{POOL_FEE, POOL_NFT_POLICY_ID, POOL_VALIDITY_POLICY_ID};
use crate::amm::pool::CFMMPool;

pub fn pool_from_utxo(mut output: BabbageTransactionOutput, output_ref: OutputRef) -> Option<CFMMPool> {
    let value = output.value_mut();
    let pool_nft_name = AssetName::from(
        value
            .multiasset
            .remove(&POOL_NFT_POLICY_ID)?
            .iter()
            .next()?
            .0
            .clone(),
    );
    value.multiasset.remove(&POOL_VALIDITY_POLICY_ID)?;
    let ((asset_x, reserves_x), (asset_y, reserves_y)) = if value.multiasset.len() == 1 {
        let (asset_y_policy, mut assets) = value.multiasset.pop_front()?;
        let (asset_y_name, reserves_y) = assets.pop_front()?;
        (
            (AssetClass::Native, value.coin),
            (
                AssetClass::Token((asset_y_policy, AssetName::from(asset_y_name))),
                reserves_y,
            ),
        )
    } else if value.multiasset.len() == 2 {
        let (asset_x_policy, mut assets) = value.multiasset.pop_front()?;
        let (asset_x_name, reserves_x) = assets.pop_front()?;
        let (asset_y_policy, mut assets) = value.multiasset.pop_front()?;
        let (asset_y_name, reserves_y) = assets.pop_front()?;
        (
            (
                AssetClass::Token((asset_x_policy, AssetName::from(asset_x_name))),
                reserves_x,
            ),
            (
                AssetClass::Token((asset_y_policy, AssetName::from(asset_y_name))),
                reserves_y,
            ),
        )
    } else {
        return None;
    };
    Some(CFMMPool {
        id: PoolId::from((*POOL_NFT_POLICY_ID, pool_nft_name)),
        version: PoolVersion::from(output_ref),
        dex: Platform::Minswap,
        reserves_x: TaggedAmount::tag(reserves_x),
        reserves_y: TaggedAmount::tag(reserves_y),
        asset_x: TaggedAssetClass::tag(asset_x),
        asset_y: TaggedAssetClass::tag(asset_y),
        lp_fee: *POOL_FEE,
    })
}
