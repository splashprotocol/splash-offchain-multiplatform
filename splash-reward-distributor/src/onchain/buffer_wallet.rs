use cml_chain::{plutus::PlutusData, Deserialize};
use cml_crypto::ScriptHash;
use spectrum_cardano_lib::{
    transaction::TransactionOutputExtension, tx_view::TxViewPartiallyResolved, AssetName, OutputRef,
};
use spectrum_offchain::{domain::Has, ledger::TryFromLedger};
use spectrum_offchain_cardano::deployment::{test_address, DeployedScriptInfo};
use splash_dao_offchain::{
    constants::SPLASH_NAME,
    protocol_config::SplashPolicy,
    routines::{Slot, TimedOutputRef},
};

use crate::onchain::RewardProtocolValidator;

/// Wallets are identified by their associated farm id, which is a CBOR-encoded integer.
#[derive(Debug, Copy, Clone, PartialEq)]
pub struct WalletId(u64);

#[derive(Debug, Clone, PartialEq)]
pub struct BufferWallet {
    id: WalletId,
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
                let mut wallet_id = None;
                let mut token_balance = 0;

                for (script_hash, hash_map) in assets {
                    let (asset_name, qty) = hash_map.iter().next().unwrap();
                    if auth_token_policy_id == *script_hash {
                        let name_pd = PlutusData::from_cbor_bytes(&asset_name.inner).ok()?;
                        if let PlutusData::Integer(i) = name_pd {
                            let id = i.as_u64()?;
                            wallet_id = Some(WalletId(id));
                        }
                    } else if splash_token_policy_id == *script_hash && *asset_name == splash_name {
                        token_balance = *qty;
                    }
                }

                if let Some(id) = wallet_id {
                    let buffer_wallet = BufferWallet { id, token_balance };
                    let timed_output_ref =
                        TimedOutputRef::new(OutputRef::new(repr.hash, ix as u64), Slot(repr.slot));
                    return Some(BufferWalletSnapshot(buffer_wallet, timed_output_ref));
                }
            }
            None
        })
    }
}
