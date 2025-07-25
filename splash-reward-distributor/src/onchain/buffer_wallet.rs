use std::fmt::Display;
use std::hash::Hash;

use cml_chain::transaction::TransactionOutput;
use cml_crypto::ScriptHash;
use derive_more::From;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use spectrum_cardano_lib::{
    transaction::TransactionOutputExtension, tx_view::TxViewPartiallyResolved, AssetName, OutputRef,
};
use spectrum_offchain::{
    domain::{EntitySnapshot, Has, Stable},
    ledger::TryFromLedger,
};
use spectrum_offchain_cardano::deployment::{test_address, DeployedScriptInfo};
use splash_dao_offchain::{
    constants::{DEFAULT_AUTH_TOKEN_NAME, SPLASH_NAME},
    protocol_config::SplashPolicy,
};

use crate::{events::EntityUpdated, onchain::RewardProtocolValidator};

#[derive(
    Copy,
    Clone,
    PartialEq,
    Eq,
    Ord,
    PartialOrd,
    From,
    Serialize,
    Deserialize,
    derive_more::Display,
    Hash,
    Debug,
)]
pub struct BufferWalletId;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BufferWallet<StateId> {
    pub state_id: StateId,
    pub balance: u64,
}

impl<StateId> Stable for BufferWallet<StateId> {
    type StableId = BufferWalletId;

    fn stable_id(&self) -> Self::StableId {
        BufferWalletId
    }

    fn is_quasi_permanent(&self) -> bool {
        true
    }
}

impl<StateId> EntitySnapshot for BufferWallet<StateId>
where
    StateId: Copy + Eq + Hash + Send + Sync + Display + Serialize + DeserializeOwned,
{
    type Version = StateId;

    fn version(&self) -> Self::Version {
        self.state_id
    }
}

#[derive(Debug, Clone)]
pub struct BufferWalletAuthToken(ScriptHash);

impl<Cx> TryFromLedger<TxViewPartiallyResolved, Cx>
    for EntityUpdated<BufferWallet<OutputRef>, OutputRef, TransactionOutput>
where
    Cx: Has<BufferWalletAuthToken>
        + Has<DeployedScriptInfo<{ RewardProtocolValidator::BufferWallet as u8 }>>
        + Has<SplashPolicy>,
{
    fn try_from_ledger(repr: &TxViewPartiallyResolved, ctx: &Cx) -> Option<Self> {
        let created = repr.outputs.iter().enumerate().find_map(|(ix, output)| {
            let output_ref = OutputRef::new(repr.hash, ix as u64);
            try_extract_buffer_wallet(output, output_ref, ctx)
                .map(|buffer_wallet| (buffer_wallet, output.clone()))
        })?;
        let consumed = repr.inputs.iter().find_map(|(tx_input, output)| {
            if let Some(output) = output {
                let output_ref = OutputRef::from(tx_input.clone());
                if try_extract_buffer_wallet(output, output_ref, ctx).is_some() {
                    return Some(output_ref);
                }
            }
            None
        });
        Some(EntityUpdated { consumed, created })
    }
}

fn try_extract_buffer_wallet<C>(
    output: &TransactionOutput,
    output_ref: OutputRef,
    ctx: &C,
) -> Option<BufferWallet<OutputRef>>
where
    C: Has<BufferWalletAuthToken>
        + Has<DeployedScriptInfo<{ RewardProtocolValidator::BufferWallet as u8 }>>
        + Has<SplashPolicy>,
{
    if test_address(output.address(), ctx) {
        let auth_token_policy_id = ctx.select::<BufferWalletAuthToken>().0;
        let splash_token_policy_id = ctx.select::<SplashPolicy>().0;
        let splash_name = cml_chain::assets::AssetName::from(AssetName::from_utf8(SPLASH_NAME.into()));
        let assets = output.value().multiasset.iter();
        let mut auth_token_found = false;
        let mut token_balance = 0;

        for (script_hash, hash_map) in assets {
            let (asset_name, qty) = hash_map.iter().next().unwrap();
            let expected_asset_name =
                cml_chain::assets::AssetName::new(DEFAULT_AUTH_TOKEN_NAME.to_be_bytes().to_vec()).unwrap();
            if auth_token_policy_id == *script_hash && *asset_name == expected_asset_name {
                auth_token_found = true;
            } else if splash_token_policy_id == *script_hash && *asset_name == splash_name {
                token_balance = *qty;
            }
        }

        if auth_token_found {
            let buffer_wallet = BufferWallet {
                balance: token_balance,
                state_id: output_ref,
            };
            return Some(buffer_wallet);
        }
    }
    None
}
