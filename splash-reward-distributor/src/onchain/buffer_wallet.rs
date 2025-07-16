use cml_chain::{plutus::PlutusData, Deserialize};
use cml_core::serialization::FromBytes;
use cml_crypto::ScriptHash;
use spectrum_cardano_lib::{
    transaction::TransactionOutputExtension, tx_view::TxViewPartiallyResolved, AssetName, OutputRef,
};
use spectrum_offchain::{domain::Has, ledger::TryFromLedger};
use spectrum_offchain_cardano::deployment::{test_address, DeployedScriptInfo};
use splash_dao_offchain::{
    constants::{DEFAULT_AUTH_TOKEN_NAME, SPLASH_NAME},
    protocol_config::SplashPolicy,
    routines::{Slot, TimedOutputRef},
};

use crate::onchain::RewardProtocolValidator;

#[derive(Debug, Clone, PartialEq)]
pub struct BufferWallet {
    token_balance: u64,
}

pub struct BufferWalletSnapshot(pub BufferWallet, pub TimedOutputRef);

#[derive(Debug, Clone)]
pub struct BufferWalletAuthToken(ScriptHash);

impl<Cx> TryFromLedger<TxViewPartiallyResolved, Cx> for BufferWalletSnapshot
where
    Cx: Has<BufferWalletAuthToken>
        + Has<DeployedScriptInfo<{ RewardProtocolValidator::WalletBuffer as u8 }>>
        + Has<SplashPolicy>,
{
    fn try_from_ledger(repr: &TxViewPartiallyResolved, ctx: &Cx) -> Option<Self> {
        repr.outputs.iter().enumerate().find_map(|(ix, output)| {
            if test_address(output.address(), ctx) {
                let auth_token_policy_id = ctx.select::<BufferWalletAuthToken>().0;
                let splash_token_policy_id = ctx.select::<SplashPolicy>().0;
                let splash_name =
                    cml_chain::assets::AssetName::from(AssetName::from_utf8(SPLASH_NAME.into()));
                let assets = output.value().multiasset.iter();
                let mut auth_token_found = false;
                let mut token_balance = 0;

                for (script_hash, hash_map) in assets {
                    let (asset_name, qty) = hash_map.iter().next().unwrap();
                    let expected_asset_name =
                        cml_chain::assets::AssetName::new(DEFAULT_AUTH_TOKEN_NAME.to_be_bytes().to_vec())
                            .unwrap();
                    if auth_token_policy_id == *script_hash && *asset_name == expected_asset_name {
                        auth_token_found = true;
                    } else if splash_token_policy_id == *script_hash && *asset_name == splash_name {
                        token_balance = *qty;
                    }
                }

                if auth_token_found {
                    let buffer_wallet = BufferWallet { token_balance };
                    let timed_output_ref =
                        TimedOutputRef::new(OutputRef::new(repr.hash, ix as u64), Slot(repr.slot));
                    return Some(BufferWalletSnapshot(buffer_wallet, timed_output_ref));
                }
            }
            None
        })
    }
}
