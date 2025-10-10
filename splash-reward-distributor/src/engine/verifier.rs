use crate::accounts::{AccountReward, Accounts};
use crate::engine::resolved_tx::PartiallySignedCardanoTx;
use crate::entity_index::{AuthManagerIndex, HarvestOrderIndex, UnconfirmedHarvestTxIndex};
use cml_chain::builders::tx_builder::SignedTxBuilder;
use cml_chain::certs::Credential;
use cml_chain::crypto::Vkeywitness;
use cml_chain::transaction::Transaction;
use cml_crypto::RawBytesEncoding;
use cml_crypto::{Ed25519KeyHash, PublicKey, TransactionHash};
use log::info;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use spectrum_cardano_lib::output::FinalizedTxOut;
use spectrum_cardano_lib::tx_view::{TimedOutput, TxViewPartiallyResolved};
use spectrum_cardano_lib::{NetworkId, OutputRef};
use spectrum_offchain::data::circular_filter::CircularFilter;
use spectrum_offchain::domain::Has;
use spectrum_offchain::ledger::TryFromLedger;
use spectrum_offchain::tx_hash::CanonicalHash;
use spectrum_offchain::tx_prover::TxProver;
use spectrum_offchain_cardano::deployment::DeployedScriptInfo;
use splash_dao_offchain::deployment::ProtocolValidator;
use splash_dao_offchain::entities::onchain::smart_farm::FarmId;
use splash_dao_offchain::protocol_config::{
    BufferWalletAuthPolicy, OperatorCreds, PermManagerAuthPolicy, SplashPolicy,
};
use splash_dao_offchain::routines::{slot_to_epoch, Slot};
use splash_dao_offchain::GenesisEpochStartTime;
use splash_yf_offchain::entities::gauge::GaugeWithdrawals;
use splash_yf_offchain::entities::{SplashTokenDecrease, SplashTokenIncrease};
use splash_yf_offchain::events::{OnChainEvent, SplashPayout};
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
    fn confirm_block_slot(&mut self, block_slot: u64);
    fn rollback_block_slot(&mut self, block_slot: u64);
}

#[derive(Clone)]
pub struct Verifier<Tx, Index, PositionIndex, UHarvestIndex, Prover> {
    index: Index,
    position_index: PositionIndex,
    unconfirmed_harvest_tx_index: UHarvestIndex,
    block_slot_buffer: CircularFilter<100, u64>,
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
            block_slot_buffer: CircularFilter::new(),
            prover,
            pd: PhantomData,
        }
    }

    fn get_current_slot(&self) -> u64 {
        *self.block_slot_buffer.back().expect("Block slot buffer is empty")
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

    fn confirm_block_slot(&mut self, block_slot: u64) {
        self.block_slot_buffer.add(block_slot);
    }

    fn rollback_block_slot(&mut self, block_slot: u64) {
        let rolled_back_slot = self
            .block_slot_buffer
            .pop_back()
            .expect("Block slot buffer is empty");
        assert!(
            rolled_back_slot == block_slot,
            "Rolled back block slot {} != confirmed block slot {}",
            rolled_back_slot,
            block_slot
        );
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
    Index: AuthManagerIndex<FarmId, OutputRef, FinalizedTxOut>
        + HarvestOrderIndex<OutputRef, FinalizedTxOut>
        + Send
        + Sync,
    PositionIndex: Accounts<OutputRef> + Send + Sync,
    UHarvestIndex: UnconfirmedHarvestTxIndex + Send + Sync,
    Prov: TxProver<SignedTxBuilder, Transaction> + Send + Sync,
    Ctx: Has<MinLovelacePerHarvest>
        + Has<DeployedScriptInfo<{ ProtocolValidator::HarvestOrder as u8 }>>
        + Has<DeployedScriptInfo<{ ProtocolValidator::PermManager as u8 }>>
        + Has<DeployedScriptInfo<{ ProtocolValidator::SmartFarm as u8 }>>
        + Has<DeployedScriptInfo<{ ProtocolValidator::BufferWallet as u8 }>>
        + Has<NetworkId>
        + Has<GenesisEpochStartTime>
        + Has<PermManagerAuthPolicy>
        + Has<SplashPolicy>
        + Has<OperatorCreds>
        + Has<AuthorizedExecutors>
        + Has<BufferWalletAuthPolicy>
        + Sync,
{
    async fn try_approve(&mut self, tx: &TxCosignRequest, ctx: &Ctx) -> Option<Transaction> {
        let current_slot = self.get_current_slot();
        match tx {
            TxCosignRequest::Harvest(tx) => {
                // Check signature
                let tx_hash = tx.tx.clone().build_checked().unwrap().canonical_hash();

                let valid_tx_signature = {
                    let vkeys = &tx.tx.witness_set().vkeys;

                    if vkeys.len() == 1 {
                        let Vkeywitness {
                            vkey,
                            ed25519_signature,
                            ..
                        } = vkeys.values().next().unwrap();

                        let authorized_signers = ctx.select::<AuthorizedExecutors>().0;
                        vkey.verify(tx_hash.to_raw_bytes(), ed25519_signature)
                            && authorized_signers.contains(vkey)
                    } else {
                        false
                    }
                };

                if !valid_tx_signature {
                    return None;
                }

                let tx_view = to_tx_view_partially_resolved(tx, current_slot);
                if let Some(OnChainEvent::BotHarvestingAction {
                    payouts,
                    buffer_wallet_update,
                    buffer_wallet_withdrawn_amount: SplashTokenDecrease(bw_withdrawn_amount),
                    ..
                }) = OnChainEvent::try_from_ledger(&tx_view, ctx)
                {
                    let current_epoch = Epoch::from(
                        slot_to_epoch(
                            current_slot,
                            ctx.select::<GenesisEpochStartTime>(),
                            ctx.select::<NetworkId>(),
                        )
                        .0 as u64,
                    );

                    let mut total_payout = 0;
                    for (order, SplashPayout(payout)) in &payouts {
                        total_payout += *payout;
                        assert_eq!(order.issued_at.1, current_epoch);
                        let last_harvested_epoch = self
                            .index
                            .last_epoch_harvested(order.account_key)
                            .await
                            .unwrap_or(Epoch::from(0));
                        if let Some(AccountReward {
                            accumulated_amount: amount,
                            latest_epoch_inclusive,
                        }) = self
                            .position_index
                            .query_account_reward(
                                &Credential::new_pub_key(order.account_key),
                                last_harvested_epoch,
                            )
                            .await
                        {
                            if amount != *payout {
                                info!(
                                "Accumulated reward amount {} (determined from lp-indexer) != harvest withdrawal amount {}",
                                amount, *payout
                            );
                                return None;
                            }
                            if current_epoch > latest_epoch_inclusive.next() {
                                info!(
                                    "lp-indexer chain tip (epoch = {}) lags current epoch ({})",
                                    latest_epoch_inclusive.next(),
                                    current_epoch,
                                );
                                return None;
                            }
                        } else {
                            return None;
                        }
                    }
                    // Check that all payouts are withdrawn from the buffer wallet
                    assert_eq!(total_payout, bw_withdrawn_amount);

                    let consumed_buffer_wallet_tx_hash = buffer_wallet_update.consumed?.tx_hash();
                    if self.unconfirmed_harvest_tx_index.try_add_tx(
                        consumed_buffer_wallet_tx_hash,
                        tx_hash,
                        payouts.iter().map(|(order, _)| order.account_key).collect(),
                    ) {
                        return Some(self.prover.prove(tx.tx.clone()));
                    }
                }
            }
            TxCosignRequest::GaugeBuffering(tx) => {
                let suspended_gauges = self.index.get_auth_manager().await?.0.suspended_gauges;

                // Check signature
                let tx_hash = tx.tx.clone().build_checked().unwrap().canonical_hash();

                let valid_tx_signature = {
                    let vkeys = &tx.tx.witness_set().vkeys;

                    if vkeys.len() == 1 {
                        let Vkeywitness {
                            vkey,
                            ed25519_signature,
                            ..
                        } = vkeys.values().next().unwrap();

                        let authorized_signers = ctx.select::<AuthorizedExecutors>().0;
                        vkey.verify(tx_hash.to_raw_bytes(), ed25519_signature)
                            && authorized_signers.contains(vkey)
                    } else {
                        false
                    }
                };

                if !valid_tx_signature {
                    return None;
                }

                let tx_view = to_tx_view_partially_resolved(tx, current_slot);
                if let Some(OnChainEvent::BotGaugeBufferingAction {
                    drained_gauges: GaugeWithdrawals(drained_gauges),
                    buffer_wallet_deposited_amount: SplashTokenIncrease(bw_deposited_amount),
                    ..
                }) = OnChainEvent::try_from_ledger(&tx_view, ctx)
                {
                    let mut total_rewards = 0;
                    for (gauge_update, SplashTokenDecrease(amount)) in &drained_gauges {
                        // Check none of the gauges are suspended
                        if suspended_gauges.contains(&gauge_update.created.0.id) {
                            return None;
                        }
                        total_rewards += *amount;
                    }

                    // Check that all rewards are only deposited in the buffer wallet
                    assert_eq!(total_rewards, bw_deposited_amount);
                    return Some(self.prover.prove(tx.tx.clone()));
                }
            }
        }
        None
    }
}

fn to_tx_view_partially_resolved(
    partially_signed_tx: &PartiallySignedCardanoTx,
    slot: u64,
) -> TxViewPartiallyResolved {
    let built_tx = partially_signed_tx.tx.clone().build_checked().unwrap();
    let hash = built_tx.canonical_hash();
    let inputs: Vec<_> = partially_signed_tx
        .inputs
        .iter()
        .zip(built_tx.body.inputs)
        .map(|(cardano_tx_input, input)| {
            let timed_output = cardano_tx_input.issued_at.map(|(slot, _)| TimedOutput {
                output: cardano_tx_input.tx_output.clone(),
                slot: slot.0,
            });

            (input, timed_output)
        })
        .collect();
    let signers = built_tx
        .witness_set
        .vkeywitnesses
        .iter()
        .flat_map(|vks| vks.iter().map(|vk| vk.vkey.hash()))
        .collect();
    TxViewPartiallyResolved {
        hash,
        inputs,
        outputs: built_tx.body.outputs,
        signers,
        slot,
    }
}
