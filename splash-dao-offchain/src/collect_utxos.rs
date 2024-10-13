use std::{cmp::Ordering, collections::HashMap};

use cml_chain::{
    builders::{
        input_builder::{InputBuilderResult, SingleInputBuilder},
        tx_builder::TransactionUnspentOutput,
    },
    Coin,
};
use spectrum_cardano_lib::{collateral::Collateral, transaction::TransactionOutputExtension, OutputRef};

use crate::deployment::BuiltPolicy;

pub fn collect_utxos(
    all_utxos: Vec<TransactionUnspentOutput>,
    required_coin: Coin,
    required_tokens: Vec<BuiltPolicy>,
    collateral: &Collateral,
) -> Vec<InputBuilderResult> {
    if required_tokens.is_empty() {
        return collect_utxos_with_no_assets(all_utxos, required_coin, collateral);
    }
    let mut res = vec![];
    let mut lovelaces_collected = 0;

    let mut skipped_utxos = vec![];

    // Maintains count of tokens whose required
    let mut count_satisfied_tokens = 0;

    let mut token_count = HashMap::new();

    for utxo in all_utxos {
        let output_ref = OutputRef::new(utxo.input.transaction_id, utxo.input.index);
        if output_ref != collateral.reference() {
            let mut add_utxo = false;
            if count_satisfied_tokens < required_tokens.len() {
                for (policy_id, name_map) in utxo.output.value().multiasset.iter() {
                    for (name, &quantity) in name_map.iter() {
                        if let Some(bp) = required_tokens
                            .iter()
                            .find(|bp| bp.policy_id == *policy_id && *name == bp.asset_name)
                        {
                            let quantity_collected = token_count.entry(output_ref).or_insert(0_u64);
                            let quantity_required = bp.quantity.as_u64().unwrap();
                            if *quantity_collected < quantity_required {
                                *quantity_collected += quantity;
                                add_utxo = true;
                                lovelaces_collected += utxo.output.amount().coin;
                                if *quantity_collected >= quantity_required {
                                    count_satisfied_tokens += 1;
                                }
                            }
                        }
                    }
                }
                if add_utxo {
                    let input_builder = SingleInputBuilder::new(utxo.input, utxo.output)
                        .payment_key()
                        .unwrap();
                    res.push(input_builder);
                } else {
                    skipped_utxos.push(utxo);
                }
            }
        }
    }

    if lovelaces_collected >= required_coin {
        return res;
    }

    // Here we've got all the required tokens but haven't met the required amount of lovelaces.
    // First sort UTxOs by coin, largest-to-smallest then select until target is met.
    skipped_utxos.sort_by_key(|u| u.output.amount().coin);
    while let Some(utxo) = skipped_utxos.pop() {
        lovelaces_collected += utxo.output.amount().coin;
        let input_builder = SingleInputBuilder::new(utxo.input, utxo.output)
            .payment_key()
            .unwrap();
        res.push(input_builder);
        if lovelaces_collected >= required_coin {
            break;
        }
    }

    res
}

fn collect_utxos_with_no_assets(
    mut all_utxos: Vec<TransactionUnspentOutput>,
    required_coin: Coin,
    collateral: &Collateral,
) -> Vec<InputBuilderResult> {
    let mut res = vec![];
    let mut lovelaces_collected = 0;

    // We choose inputs with the fewest number of tokens and also the smallest ADA balances, to keep
    // the number of UTxOs in the wallet down.
    all_utxos.sort_by(|a, b| {
        let num_tokens_a = a.output.amount().multiasset.len();
        let num_tokens_b = b.output.amount().multiasset.len();
        if num_tokens_a < num_tokens_b {
            Ordering::Less
        } else if num_tokens_a == num_tokens_b {
            a.output.amount().coin.cmp(&b.output.amount().coin)
        } else {
            Ordering::Greater
        }
    });

    for utxo in all_utxos {
        let output_ref = OutputRef::new(utxo.input.transaction_id, utxo.input.index);
        if output_ref != collateral.reference() {
            if lovelaces_collected > required_coin {
                break;
            }

            let coin = utxo.output.amount().coin;
            lovelaces_collected += coin;
            let input_builder = SingleInputBuilder::new(utxo.input, utxo.output)
                .payment_key()
                .unwrap();
            res.push(input_builder);
        }
    }
    res
}
