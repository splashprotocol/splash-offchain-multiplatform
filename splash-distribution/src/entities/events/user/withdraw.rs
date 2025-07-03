use crate::distributor::user_requests_processor::splash_pair_id;
use cml_chain::address::{Address, EnterpriseAddress};
use cml_chain::assets::{MultiAsset, PositiveCoin};
use cml_chain::certs::StakeCredential;
use cml_chain::plutus::PlutusData;
use cml_chain::transaction::{ConwayFormatTxOut, TransactionOutput};
use cml_chain::{PolicyId, Value};
use cml_core::serialization::RawBytesEncoding;
use cml_crypto::Ed25519KeyHash;
use log::info;
use serde::{Deserialize, Serialize};
use spectrum_cardano_lib::plutus_data::{ConstrPlutusDataExtension, DatumExtension, PlutusDataExtension};
use spectrum_cardano_lib::transaction::TransactionOutputExtension;
use spectrum_cardano_lib::types::TryFromPData;
use spectrum_cardano_lib::NetworkId;
use spectrum_offchain::domain::Has;
use spectrum_offchain::ledger::{IntoLedger, TryFromLedger};
use spectrum_offchain_cardano::data::pair::PairId;
use spectrum_offchain_cardano::deployment::{test_address, DeployedScriptInfo};
use splash_dao_offchain::create_change_output::Token;
use splash_dao_offchain::deployment::ProtocolValidator;
use splash_dao_offchain::entities::HasStatus;
use splash_dao_offchain::entities::onchain::voting_escrow::{Lock, Owner};
use splash_dao_offchain::routines::Slot;
use splash_lp_indexer::config::HarvestLimits;
use splash_lp_indexer::onchain::event::WithOptionalSlot;

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Debug, Hash)]
pub struct UserWithdrawConfig {
    pub refund_key: Ed25519KeyHash,
    pub distribution_agent_key: Ed25519KeyHash,
}

impl TryFromPData for UserWithdrawConfig {
    fn try_from_pd(data: PlutusData) -> Option<Self> {
        let mut cpd = data.into_constr_pd()?;
        let refund_key = Ed25519KeyHash::from_raw_bytes(cpd.take_field(0)?.into_bytes()?.as_ref()).ok()?;
        let distribution_agent_key =
            Ed25519KeyHash::from_raw_bytes(cpd.take_field(1)?.into_bytes()?.as_ref()).ok()?;
        Some(UserWithdrawConfig {
            refund_key,
            distribution_agent_key,
        })
    }
}

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct UserWithdraw {
    pub status: UserWithdrawStatus,
    // todo: verify that cml::Address corresponds to lp_indexer account
    pub account: Address,
}

#[derive(Clone, Copy, Serialize, Deserialize, Debug)]
pub enum UserWithdrawStatus {
    New,
    InProgress,
    LimitReached,
    Approved,
    SendToBlockchain,
    AwaitConfirmation,
    Withdrawn
}

impl WithOptionalSlot for UserWithdraw {
    fn slot(&self) -> Option<Slot> {
        None
    }
}

impl UserWithdraw {
    pub fn set_in_progress(&mut self) -> () {
        self.status = UserWithdrawStatus::InProgress
    }

    pub fn mark_as_processed(&mut self) -> () {
        self.status = UserWithdrawStatus::SendToBlockchain
    }

    pub fn mark_as_new(&mut self) -> () {
        self.status = UserWithdrawStatus::New
    }

    pub fn finalize_withdraw(&self, distributed_splash: u64) -> TransactionOutput {
        info!("finalize_withdraw");

        // todo: provide correct ada value
        let ada_value = 1_000_000;

        let mut ma = MultiAsset::new();
        ma.set(splash_pair_id().0, splash_pair_id().1.into(), distributed_splash);

        TransactionOutput::new_conway_format_tx_out(ConwayFormatTxOut {
            address: self.account.clone(),
            amount: Value::new(ada_value, ma),
            datum_option: None,
            script_reference: None,
            encodings: None,
        })
    }
}

impl HasStatus for UserWithdraw {
    type Status = UserWithdrawStatus;

    fn get_status(self) -> Self::Status {
        self.status
    }
}

impl<C> TryFromLedger<TransactionOutput, C> for UserWithdraw
where
    C: Clone
        + Has<DeployedScriptInfo<{ ProtocolValidator::HarvestOrder as u8 }>>
        + Has<HarvestLimits>
        + Has<NetworkId>,
{
    fn try_from_ledger(repr: &TransactionOutput, ctx: &C) -> Option<Self> {
        if test_address(repr.address(), ctx) {
            // let value = repr.value().clone();
            let conf = UserWithdrawConfig::try_from_pd(repr.datum()?.into_pd()?)?;
            let network_id: NetworkId = ctx.get();
            return Some(UserWithdraw {
                status: UserWithdrawStatus::New,
                account: Address::Enterprise(EnterpriseAddress::new(
                    network_id.into(),
                    StakeCredential::new_pub_key(conf.refund_key),
                )),
            });
        }
        None
    }
}

impl<Ctx> IntoLedger<TransactionOutput, Ctx> for UserWithdraw {
    fn into_ledger(self, ctx: Ctx) -> TransactionOutput {
        info!("IntoLedger UserWithdraw");

        // todo: provide correct ada value
        let ada_value = 1_000_000;

        let mut ma = MultiAsset::new();
        //ma.set(splash_pair_id().0, splash_pair_id().1.into(), distributed_splash);

        TransactionOutput::new_conway_format_tx_out(ConwayFormatTxOut {
            address: self.account.clone(),
            amount: Value::new(ada_value, ma),
            datum_option: None,
            script_reference: None,
            encodings: None,
        })
    }
}
