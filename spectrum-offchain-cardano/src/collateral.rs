use cml_chain::address::Address;
use cml_chain::builders::tx_builder::TransactionUnspentOutput;

use cardano_explorer::CardanoNetwork;
use spectrum_cardano_lib::collateral::Collateral;
use spectrum_cardano_lib::transaction::TransactionOutputExtension;

use crate::constants::MIN_SAFE_COLLATERAL;

const LIMIT: u16 = 50;

pub async fn pull_collateral<Net: CardanoNetwork>(
    collateral_address: Address,
    explorer: &Net,
) -> Option<Collateral> {
    let mut collateral: Option<TransactionUnspentOutput> = None;
    let mut offset = 0u32;
    while collateral.is_none() {
        let utxos = explorer
            .utxos_by_address(collateral_address.clone(), offset, LIMIT)
            .await;
        if utxos.is_empty() {
            break;
        }
        if let Some(x) = utxos
            .into_iter()
            .find(|u| !u.output.amount().has_multiassets() && u.output.value().coin >= MIN_SAFE_COLLATERAL)
        {
            collateral = Some(x);
        }
        offset += LIMIT as u32;
    }
    collateral.map(|out| out.into())
}
