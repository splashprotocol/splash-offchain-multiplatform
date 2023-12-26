use cml_chain::address::{BaseAddress, EnterpriseAddress};
use cml_chain::assets::MultiAsset;
use cml_chain::certs::StakeCredential;
use cml_chain::genesis::network_info::NetworkInfo;
use cml_chain::transaction::{ConwayFormatTxOut, TransactionOutput};
use cml_chain::{Coin, Value};
use cml_crypto::Ed25519KeyHash;

use crate::data::execution_context::ExecutionContext;
use spectrum_cardano_lib::{TaggedAmount, TaggedAssetClass};
use spectrum_offchain::ledger::IntoLedger;

use crate::data::order::Quote;
use crate::data::pool::{Lq, Rx, Ry};

#[derive(Debug, Clone)]
pub struct SwapOutput {
    pub quote_asset: TaggedAssetClass<Quote>,
    pub quote_amount: TaggedAmount<Quote>,
    pub ada_residue: Coin,
    pub redeemer_pkh: Ed25519KeyHash,
    pub redeemer_stake_pkh: Option<Ed25519KeyHash>,
}

impl IntoLedger<TransactionOutput, ExecutionContext> for SwapOutput {
    fn into_ledger(self, ctx: ExecutionContext) -> TransactionOutput {
        let addr = if let Some(stake_pkh) = self.redeemer_stake_pkh {
            BaseAddress::new(
                ctx.network_id,
                StakeCredential::new_pub_key(self.redeemer_pkh),
                StakeCredential::new_pub_key(stake_pkh),
            )
            .to_address()
        } else {
            EnterpriseAddress::new(ctx.network_id, StakeCredential::new_pub_key(self.redeemer_pkh))
                .to_address()
        };

        let mut ma = MultiAsset::new();

        let ada_from_quote = if self.quote_asset.is_native() {
            self.quote_amount.untag()
        } else {
            let (policy, name) = self.quote_asset.untag().into_token().unwrap();
            ma.set(policy, name.into(), self.quote_amount.untag());
            0
        };

        let ada = self.ada_residue + ada_from_quote;

        TransactionOutput::new_conway_format_tx_out(ConwayFormatTxOut {
            address: addr,
            amount: Value::new(ada, ma),
            datum_option: None,
            script_reference: None,
            encodings: None,
        })
    }
}

#[derive(Debug, Clone)]
pub struct DepositOutput {
    pub token_x_asset: TaggedAssetClass<Rx>,
    pub token_x_charge_amount: TaggedAmount<Rx>,
    pub token_y_asset: TaggedAssetClass<Ry>,
    pub token_y_charge_amount: TaggedAmount<Ry>,
    pub token_lq_asset: TaggedAssetClass<Lq>,
    pub token_lq_amount: TaggedAmount<Lq>,
    pub ada_residue: Coin,
    pub redeemer_pkh: Ed25519KeyHash,
    pub redeemer_stake_pkh: Option<Ed25519KeyHash>,
}

impl IntoLedger<TransactionOutput, ExecutionContext> for DepositOutput {
    fn into_ledger(self, ctx: ExecutionContext) -> TransactionOutput {
        let addr = if let Some(stake_pkh) = self.redeemer_stake_pkh {
            BaseAddress::new(
                ctx.network_id,
                StakeCredential::new_pub_key(self.redeemer_pkh),
                StakeCredential::new_pub_key(stake_pkh),
            )
            .to_address()
        } else {
            EnterpriseAddress::new(ctx.network_id, StakeCredential::new_pub_key(self.redeemer_pkh))
                .to_address()
        };

        let mut ma = MultiAsset::new();

        let ada_from_charge_pair = match (self.token_x_asset.is_native(), self.token_y_asset.is_native()) {
            (true, false) => {
                let (policy, name) = self.token_y_asset.untag().into_token().unwrap();
                ma.set(policy, name.into(), self.token_y_charge_amount.untag());
                self.token_x_charge_amount.untag()
            }
            (false, true) => {
                let (policy, name) = self.token_x_asset.untag().into_token().unwrap();
                ma.set(policy, name.into(), self.token_x_charge_amount.untag());
                self.token_y_charge_amount.untag()
            }
            (false, false) => {
                let (policy_x, name_x) = self.token_x_asset.untag().into_token().unwrap();
                ma.set(policy_x, name_x.into(), self.token_x_charge_amount.untag());
                let (policy_y, name_y) = self.token_y_asset.untag().into_token().unwrap();
                ma.set(policy_y, name_y.into(), self.token_y_charge_amount.untag());
                0
            }
            // todo: basically this is unreachable point. Throw error?
            (true, true) => self.token_x_charge_amount.untag() + self.token_y_charge_amount.untag(),
        };

        let ada = self.ada_residue + ada_from_charge_pair;

        TransactionOutput::new_conway_format_tx_out(ConwayFormatTxOut {
            address: addr,
            amount: Value::new(ada, ma),
            datum_option: None,
            script_reference: None,
            encodings: None,
        })
    }
}

#[derive(Debug, Clone)]
pub struct RedeemOutput {
    pub token_x_asset: TaggedAssetClass<Rx>,
    pub token_x_amount: TaggedAmount<Rx>,
    pub token_y_asset: TaggedAssetClass<Ry>,
    pub token_y_amount: TaggedAmount<Ry>,
    pub ada_residue: Coin,
    pub redeemer_pkh: Ed25519KeyHash,
    pub redeemer_stake_pkh: Option<Ed25519KeyHash>,
}

impl IntoLedger<TransactionOutput, ExecutionContext> for RedeemOutput {
    fn into_ledger(self, ctx: ExecutionContext) -> TransactionOutput {
        let addr = if let Some(stake_pkh) = self.redeemer_stake_pkh {
            BaseAddress::new(
                ctx.network_id,
                StakeCredential::new_pub_key(self.redeemer_pkh),
                StakeCredential::new_pub_key(stake_pkh),
            )
            .to_address()
        } else {
            EnterpriseAddress::new(ctx.network_id, StakeCredential::new_pub_key(self.redeemer_pkh))
                .to_address()
        };

        let mut ma = MultiAsset::new();

        let ada_from_charge_pair = match (self.token_x_asset.is_native(), self.token_y_asset.is_native()) {
            (true, false) => {
                let (policy, name) = self.token_y_asset.untag().into_token().unwrap();
                ma.set(policy, name.into(), self.token_y_amount.untag());
                self.token_x_amount.untag()
            }
            (false, true) => {
                let (policy, name) = self.token_x_asset.untag().into_token().unwrap();
                ma.set(policy, name.into(), self.token_x_amount.untag());
                self.token_y_amount.untag()
            }
            (false, false) => {
                let (policy_x, name_x) = self.token_x_asset.untag().into_token().unwrap();
                ma.set(policy_x, name_x.into(), self.token_x_amount.untag());
                let (policy_y, name_y) = self.token_y_asset.untag().into_token().unwrap();
                ma.set(policy_y, name_y.into(), self.token_y_amount.untag());
                0
            }
            // todo: basically this is unreachable point. Throw error?
            (true, true) => self.token_x_amount.untag() + self.token_y_amount.untag(),
        };

        let ada = self.ada_residue + ada_from_charge_pair;

        TransactionOutput::new_conway_format_tx_out(ConwayFormatTxOut {
            address: addr,
            amount: Value::new(ada, ma),
            datum_option: None,
            script_reference: None,
            encodings: None,
        })
    }
}
