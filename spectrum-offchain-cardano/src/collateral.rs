use cml_chain::builders::tx_builder::TransactionUnspentOutput;

use cardano_explorer::CardanoNetwork;
use spectrum_cardano_lib::collateral::Collateral;
use spectrum_cardano_lib::transaction::TransactionOutputExtension;

use crate::constants::MIN_SAFE_COLLATERAL;
use crate::creds::CollateralAddress;

const LIMIT: u16 = 50;

pub async fn pull_collateral<Net: CardanoNetwork>(
    collateral_address: CollateralAddress,
    explorer: &Net,
) -> Option<Collateral> {
    let mut collateral: Option<TransactionUnspentOutput> = None;
    let mut offset = 0u32;
    let mut num_utxos_pulled = 0;
    while collateral.is_none() {
        let utxos = explorer
            .utxos_by_address(collateral_address.clone().address(), offset, LIMIT)
            .await;
        if utxos.is_empty() {
            break;
        }
        if utxos.len() > num_utxos_pulled {
            num_utxos_pulled = utxos.len();
        } else {
            // Didn't find any new UTxOs
            break;
        }
        if let Some(x) = utxos.into_iter().find(|u| {
            !u.output.amount().has_multiassets() && u.output.value().coin == MIN_SAFE_COLLATERAL * 2
        }) {
            collateral = Some(x);
        }
        offset += LIMIT as u32;
    }
    collateral.map(|out| out.into())
}
