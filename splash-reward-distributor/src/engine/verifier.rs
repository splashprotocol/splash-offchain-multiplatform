use crate::accounts::{AccountReward, Accounts};
use crate::engine::proposed_harvest_tx::{ProposedHarvestTx, Withdrawal};
use crate::engine::resolved_tx::PartiallySignedCardanoTx;
use crate::entity_index::{AuthManagerIndex, UnconfirmedHarvestTxIndex};
use cml_chain::builders::tx_builder::SignedTxBuilder;
use cml_chain::certs::Credential;
use cml_chain::transaction::Transaction;
use cml_crypto::{Ed25519KeyHash, PublicKey, TransactionHash};
use log::info;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use spectrum_cardano_lib::output::FinalizedTxOut;
use spectrum_cardano_lib::{NetworkId, OutputRef};
use spectrum_offchain::domain::Has;
use spectrum_offchain::ledger::TryFromLedger;
use spectrum_offchain::tx_hash::CanonicalHash;
use spectrum_offchain::tx_prover::TxProver;
use spectrum_offchain_cardano::deployment::DeployedScriptInfo;
use splash_dao_offchain::deployment::ProtocolValidator;
use splash_dao_offchain::entities::onchain::smart_farm::FarmId;
use splash_dao_offchain::protocol_config::{BufferWalletScript, SplashPolicy};
use splash_yf_offchain::settings::MinLovelacePerHarvest;
use splash_yf_offchain::Epoch;
use std::collections::HashSet;
use std::marker::PhantomData;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VerifierRejection {
    InvalidWithdrawal,
    Unavailable,
}

#[derive(Debug, Serialize, Deserialize)]
struct VerificationRequest {
    tx_data: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct VerificationResponse {
    approved: bool,
}

#[derive(Clone)]
pub struct HttpVerifier {
    client: Client,
    verification_url: String,
}

impl HttpVerifier {
    pub fn new(verification_url: String) -> Self {
        Self {
            verification_url,
            client: Client::new(),
        }
    }
}

#[async_trait::async_trait]
pub trait RemoteVerifier<PartialTx, Tx> {
    async fn try_approve(&self, tx: &PartialTx) -> Result<Tx, VerifierRejection>;
}

#[async_trait::async_trait]
impl RemoteVerifier<PartiallySignedCardanoTx, Transaction> for HttpVerifier {
    async fn try_approve(&self, tx: &PartiallySignedCardanoTx) -> Result<Transaction, VerifierRejection> {
        let response = self
            .client
            .post(&self.verification_url)
            .json(tx)
            .send()
            .await
            .map_err(|_| VerifierRejection::Unavailable)?;

        let cosigned_tx: Transaction = response
            .json()
            .await
            .map_err(|_| VerifierRejection::Unavailable)?;

        Ok(cosigned_tx)
    }
}

#[async_trait::async_trait]
pub trait LocalVerifier<PartialTx, Tx, Ctx> {
    async fn try_approve(&mut self, tx: &PartialTx, ctx: &Ctx) -> Option<Tx>;
}

pub trait VerifierHandleLedgerEvent {
    fn confirm_harvest_tx(&mut self, tx_hash: TransactionHash, confirmed_user_harvests: &[Ed25519KeyHash]);
    fn confirm_gauge_buffering_tx(&mut self, tx_hash: TransactionHash);
    fn rollback_harvest_tx(&mut self, tx_hash: TransactionHash, confirmed_user_harvests: Vec<Ed25519KeyHash>);
    fn rollback_gauge_buffering_tx(&mut self, tx_hash: TransactionHash);
}

#[derive(Clone)]
pub struct Verifier<Tx, Index, PositionIndex, UHarvestIndex, Prover> {
    index: Index,
    position_index: PositionIndex,
    unconfirmed_harvest_tx_index: UHarvestIndex,
    prover: Prover,
    pd: PhantomData<Tx>,
}

impl<Index, PositionIndex, UHarvestIndex, Prover>
    Verifier<TxCosignRequest, Index, PositionIndex, UHarvestIndex, Prover>
{
    pub fn new(
        index: Index,
        position_index: PositionIndex,
        unconfirmed_harvest_tx_index: UHarvestIndex,
        prover: Prover,
    ) -> Self {
        Self {
            index,
            position_index,
            unconfirmed_harvest_tx_index,
            prover,
            pd: PhantomData,
        }
    }
}

impl<Index, PositionIndex, UHarvestIndex, Prov> VerifierHandleLedgerEvent
    for Verifier<TxCosignRequest, Index, PositionIndex, UHarvestIndex, Prov>
where
    Index: Send + Sync,
    PositionIndex: Accounts<OutputRef> + Send + Sync,
    UHarvestIndex: UnconfirmedHarvestTxIndex + Send + Sync,
    Prov: TxProver<SignedTxBuilder, Transaction> + Send + Sync,
{
    fn confirm_harvest_tx(&mut self, tx_hash: TransactionHash, confirmed_user_harvests: &[Ed25519KeyHash]) {
        self.unconfirmed_harvest_tx_index
            .confirm_tx(tx_hash, &confirmed_user_harvests.iter().cloned().collect());
    }

    fn confirm_gauge_buffering_tx(&mut self, tx_hash: TransactionHash) {
        assert!(!self
            .unconfirmed_harvest_tx_index
            .confirm_tx(tx_hash, &HashSet::new()));
    }

    fn rollback_harvest_tx(
        &mut self,
        tx_hash: TransactionHash,
        confirmed_user_harvests: Vec<Ed25519KeyHash>,
    ) {
        self.unconfirmed_harvest_tx_index
            .rollback(confirmed_user_harvests.into_iter().collect(), tx_hash);
    }

    fn rollback_gauge_buffering_tx(&mut self, tx_hash: TransactionHash) {
        self.unconfirmed_harvest_tx_index
            .rollback(HashSet::new(), tx_hash);
    }
}

#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct AuthorizedExecutors(pub Vec<PublicKey>);

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum TxCosignRequest {
    Harvest(PartiallySignedCardanoTx),
    GaugeBuffering(PartiallySignedCardanoTx),
}

#[async_trait::async_trait]
impl<Index, PositionIndex, UHarvestIndex, Prov, Ctx> LocalVerifier<TxCosignRequest, Transaction, Ctx>
    for Verifier<TxCosignRequest, Index, PositionIndex, UHarvestIndex, Prov>
where
    Index: AuthManagerIndex<FarmId, OutputRef, FinalizedTxOut> + Send + Sync,
    PositionIndex: Accounts<OutputRef> + Send + Sync,
    UHarvestIndex: UnconfirmedHarvestTxIndex + Send + Sync,
    Prov: TxProver<SignedTxBuilder, Transaction> + Send + Sync,
    Ctx: Has<MinLovelacePerHarvest>
        + Has<DeployedScriptInfo<{ ProtocolValidator::HarvestOrder as u8 }>>
        + Has<NetworkId>
        + Has<SplashPolicy>
        + Has<AuthorizedExecutors>
        + Has<BufferWalletScript>
        + Sync,
{
    async fn try_approve(&mut self, tx: &TxCosignRequest, ctx: &Ctx) -> Option<Transaction> {
        match tx {
            TxCosignRequest::Harvest(tx) => {
                let proposed_harvest_tx = <ProposedHarvestTx<OutputRef>>::try_from_ledger(tx, ctx)?;
                let withdrawals = proposed_harvest_tx.withdrawals;
                for withdrawal in &withdrawals {
                    let order = withdrawal.order.clone();
                    if let Some(AccountReward {
                        accumulated_amount: amount,
                        latest_epoch_inclusive,
                    }) = self
                        .position_index
                        .query_account_reward(&Credential::new_pub_key(order.account_key), Epoch::from(0)) // todo: use correct epoch
                        .await
                    {
                        if amount != withdrawal.amount {
                            info!(
                                "Accumulated reward amount {} != harvest withdrawal amount {}",
                                amount, withdrawal.amount
                            );
                            return None;
                        }
                        if order.issued_at.1 > latest_epoch_inclusive.next() {
                            info!(
                                "Harvest order issued at epoch {} is in the future",
                                order.issued_at.1
                            );
                            return None;
                        }
                    } else {
                        return None;
                    }
                }
                let tx_hash = tx.tx.clone().build_checked().unwrap().canonical_hash();
                if self.unconfirmed_harvest_tx_index.try_add_tx(
                    proposed_harvest_tx.buffer_wallet_tx_hash,
                    tx_hash,
                    withdrawals
                        .iter()
                        .map(|withdrawal| withdrawal.order.account_key)
                        .collect(),
                ) {
                    return Some(self.prover.prove(tx.tx.clone()));
                }
            }
            TxCosignRequest::GaugeBuffering(tx) => {
                let auth_manager = self.index.get_auth_manager().await?;
                // Check none of the gauges are suspended
                // check that all rewards are deposited in the buffer wallet
                todo!()
            }
        }
        None
    }
}
