use std::fmt::Display;
use std::hash::Hash;

use cml_chain::{certs::StakeCredential, transaction::TransactionOutput};
use cml_crypto::ScriptHash;
use derive_more::From;
use rs_merkle::{algorithms::Keccak256, MerkleTree};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use spectrum_cardano_lib::{
    output::FinalizedTxOut,
    transaction::TransactionOutputExtension,
    tx_view::{TimedOutput, TxViewPartiallyResolved},
    AssetName, OutputRef,
};
use spectrum_offchain::{
    domain::{EntitySnapshot, Has, Stable},
    ledger::TryFromLedger,
};
use spectrum_offchain_cardano::deployment::{test_address, DeployedScriptInfo};
use splash_dao_offchain::{
    constants::{DEFAULT_AUTH_TOKEN_NAME, SPLASH_NAME},
    protocol_config::{BufferWalletScript, SplashPolicy},
};

use crate::{entities::BufferWalletSplashBalanceChange, events::EntityUpdated};

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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BufferWallet<StateId> {
    pub state_id: StateId,
    pub balance: u64,
    pub merkle_tree_root_hash: [u8; 32],
}

#[derive(Clone, Serialize, Deserialize)]
pub struct BufferWalletWrap<StateId> {
    pub wallet: BufferWallet<StateId>,
    /// If the associated `BufferWallet` instance is predicted (i.e. not yet confirmed on-chain),
    /// this field will store the computed Merkle-tree.
    pub predicted_merkle_tree: Option<MerkleTree<Keccak256>>,
}

impl<StateId> Stable for BufferWalletWrap<StateId> {
    type StableId = BufferWalletId;

    fn stable_id(&self) -> Self::StableId {
        BufferWalletId
    }

    fn is_quasi_permanent(&self) -> bool {
        true
    }
}

impl<StateId> EntitySnapshot for BufferWalletWrap<StateId>
where
    StateId: Copy + Eq + Hash + Send + Sync + Display + Serialize + DeserializeOwned,
{
    type Version = StateId;

    fn version(&self) -> Self::Version {
        self.wallet.state_id
    }
}

pub struct BufferWalletUpdate<StateId, Bearer> {
    pub update: EntityUpdated<BufferWallet<StateId>, StateId, Bearer>,
    pub balance_change: BufferWalletSplashBalanceChange,
}

#[derive(Debug, Clone)]
pub struct BufferWalletAuthToken(ScriptHash);

impl<Cx> TryFromLedger<TxViewPartiallyResolved, Cx> for BufferWalletUpdate<OutputRef, FinalizedTxOut>
where
    Cx: Has<BufferWalletScript> + Has<SplashPolicy>,
{
    fn try_from_ledger(repr: &TxViewPartiallyResolved, ctx: &Cx) -> Option<Self> {
        let (created, output_balance) = repr.outputs.iter().enumerate().find_map(|(ix, output)| {
            let output_ref = OutputRef::new(repr.hash, ix as u64);
            try_extract_buffer_wallet(output, output_ref, ctx).map(|buffer_wallet| {
                let balance = buffer_wallet.balance;
                (
                    (buffer_wallet, FinalizedTxOut(output.clone(), output_ref)),
                    balance,
                )
            })
        })?;
        let consumed = repr.inputs.iter().find_map(|(tx_input, output)| {
            if let Some(TimedOutput { output, .. }) = output {
                let output_ref = OutputRef::from(tx_input.clone());
                if let Some(buffer_wallet) = try_extract_buffer_wallet(output, output_ref, ctx) {
                    return Some((output_ref, buffer_wallet.balance));
                }
            }
            None
        });
        let balance_change = if let Some((_, input_balance)) = consumed {
            BufferWalletSplashBalanceChange::from_diff(input_balance, created.0.balance)
        } else {
            BufferWalletSplashBalanceChange::from_diff(0, output_balance)
        };
        let consumed = consumed.map(|(consumed, _)| consumed);
        let update = EntityUpdated { consumed, created };
        Some(BufferWalletUpdate {
            update,
            balance_change,
        })
    }
}

pub fn try_extract_buffer_wallet<C>(
    output: &TransactionOutput,
    output_ref: OutputRef,
    ctx: &C,
) -> Option<BufferWallet<OutputRef>>
where
    C: Has<BufferWalletScript> + Has<SplashPolicy>,
{
    if let Some(StakeCredential::Script { hash, .. }) = output.address().payment_cred() {
        if *hash == ctx.select::<BufferWalletScript>().0.hash() {
            let splash_token_policy_id = ctx.select::<SplashPolicy>().0;
            let splash_name = cml_chain::assets::AssetName::from(AssetName::from_utf8(SPLASH_NAME.into()));
            let assets = output.value().multiasset.iter();
            let mut token_balance = 0;

            for (script_hash, hash_map) in assets {
                let (asset_name, qty) = hash_map.iter().next().unwrap();
                if splash_token_policy_id == *script_hash && *asset_name == splash_name {
                    token_balance = *qty;
                }
            }

            let buffer_wallet = BufferWallet {
                balance: token_balance,
                state_id: output_ref,
                merkle_tree_root_hash: [0; 32], // TODO: impl in DEX-935
            };
            return Some(buffer_wallet);
        }
    }
    None
}
