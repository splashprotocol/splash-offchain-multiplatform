use crate::accounts::{AccountReward, Accounts};
use crate::engine::resolved_tx::PartiallySignedCardanoTx;
use crate::engine::withdrawal::Withdrawal;
use cml_chain::certs::Credential;
use cml_chain::transaction::Transaction;
use cml_crypto::Ed25519KeyHash;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use spectrum_cardano_lib::{NetworkId, OutputRef};
use spectrum_offchain::domain::Has;
use spectrum_offchain::ledger::TryFromLedger;
use spectrum_offchain::tx_prover::TxProver;
use spectrum_offchain_cardano::deployment::DeployedScriptInfo;
use splash_dao_offchain::deployment::ProtocolValidator;
use splash_dao_offchain::protocol_config::{BufferWalletScript, SplashPolicy};
use splash_yf_offchain::settings::MinLovelacePerHarvest;
use std::marker::PhantomData;
use splash_yf_offchain::Epoch;

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
    async fn try_approve(&self, tx: &PartialTx, ctx: &Ctx) -> Option<Tx>;
}

pub struct Verifier<Tx, Index, Prover> {
    index: Index,
    prover: Prover,
    pd: PhantomData<Tx>,
}

#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct AuthorizedExecutors(pub Vec<Ed25519KeyHash>);

#[async_trait::async_trait]
impl<Index, Prov, Ctx> LocalVerifier<PartiallySignedCardanoTx, Transaction, Ctx>
    for Verifier<PartiallySignedCardanoTx, Index, Prov>
where
    Index: Accounts<OutputRef> + Send + Sync,
    Prov: TxProver<PartiallySignedCardanoTx, Transaction> + Send + Sync,
    Ctx: Has<MinLovelacePerHarvest>
        + Has<DeployedScriptInfo<{ ProtocolValidator::HarvestOrder as u8 }>>
        + Has<NetworkId>
        + Has<SplashPolicy>
        + Has<AuthorizedExecutors>
        + Has<BufferWalletScript>
        + Sync,
{
    async fn try_approve(&self, tx: &PartiallySignedCardanoTx, ctx: &Ctx) -> Option<Transaction> {
        if let Some(withdrawals) = <Vec<Withdrawal<OutputRef>>>::try_from_ledger(tx, ctx) {
            for withdrawal in withdrawals {
                let order = withdrawal.order;
                if let Some(AccountReward {
                              accumulated_amount: amount,
                    latest_epoch_inclusive,
                }) = self
                    .index
                    .query_account_reward(&Credential::new_pub_key(order.account_key), Epoch::from(0)) // todo: use correct epoch
                    .await
                {
                    if amount != withdrawal.amount {
                        return None;
                    }
                } else {
                    return None;
                }
            }
            return Some(self.prover.prove(tx.clone()));
        }
        None
    }
}
