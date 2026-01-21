use crate::accounts::{AccountReward, Accounts};
use crate::engine::resolved_tx::PartiallySignedCardanoTx;
use crate::entity_index::{AuthManagerIndex, HarvestOrderIndex, UnconfirmedRewardTxIndex};
use cml_chain::builders::tx_builder::SignedTxBuilder;
use cml_chain::certs::Credential;
use cml_chain::crypto::utils::make_vkey_witness;
use cml_chain::crypto::Vkeywitness;
use cml_chain::transaction::Transaction;
use cml_crypto::{Ed25519KeyHash, PublicKey, TransactionHash};
use cml_crypto::{PrivateKey, RawBytesEncoding};
use log::{error, info, trace, warn};
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
    BufferWalletAuthPolicy, FarmFactoryAuthPolicy, OperatorCreds, PermManagerAuthPolicy, SplashPolicy,
};
use splash_dao_offchain::routines::slot_to_epoch;
use splash_dao_offchain::GenesisEpochStartTime;
use splash_yf_offchain::entities::gauge::GaugeWithdrawals;
use splash_yf_offchain::entities::{BufferWalletSplashTokenDecrease, BufferWalletSplashTokenIncrease};
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
        let url = format!("http://{}/cosign", self.verification_url);
        let cosign_request = TxCosignRequest::GaugeBuffering(tx.clone());
        let tx_bytes = rmp_serde::to_vec_named(&cosign_request).unwrap();
        let response = self
            .client
            .put(&url)
            .header("Content-Type", "application/x-msgpack")
            .body(tx_bytes)
            .send()
            .await;

        match response {
            Ok(response) => {
                trace!("response: {:?}", response);
                use cml_chain::{Deserialize, Serialize};
                let cosigned_tx_hex: String = response
                    .json()
                    .await
                    .map_err(|_| VerifierRejection::Unavailable)?;
                let cosigned_tx_bytes =
                    hex::decode(cosigned_tx_hex).map_err(|_| VerifierRejection::Unavailable)?;
                let cosigned_tx = Transaction::from_cbor_bytes(&cosigned_tx_bytes)
                    .map_err(|_| VerifierRejection::Unavailable)?;
                trace!("cosigned_tx: {:?}", cosigned_tx);
                Ok(cosigned_tx)
            }
            Err(e) => {
                error!("Error verifying tx: {:?}", e);
                Err(VerifierRejection::Unavailable)
            }
        }
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
    genesis_epoch_start_time: GenesisEpochStartTime,
    network_id: NetworkId,
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
        genesis_epoch_start_time: GenesisEpochStartTime,
        network_id: NetworkId,
        prover: Prover,
    ) -> Self {
        Self {
            index,
            position_index,
            unconfirmed_harvest_tx_index,
            block_slot_buffer: CircularFilter::new(),
            genesis_epoch_start_time,
            network_id,
            prover,
            pd: PhantomData,
        }
    }

    fn get_current_slot(&self) -> Option<u64> {
        // We can be sure that `block_slot_buffer` is not empty because verification can't be
        // performed until chain-sync is complete.
        self.block_slot_buffer.back().copied()
    }

    fn compute_epoch(&self, slot: u64) -> u64 {
        slot_to_epoch(slot, self.genesis_epoch_start_time, self.network_id).0 as u64
    }
}

impl<Index, PositionIndex, UHarvestIndex, Prov> VerifierHandleLedgerEvent
    for Verifier<TxCosignRequest, Index, PositionIndex, UHarvestIndex, Prov>
where
    Index: Send + Sync,
    PositionIndex: Accounts<OutputRef> + Send + Sync,
    UHarvestIndex: UnconfirmedRewardTxIndex + Send + Sync,
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
        if let Some(current_slot) = self.get_current_slot() {
            let current_epoch = self.compute_epoch(current_slot);
            let new_epoch = self.compute_epoch(block_slot);
            if new_epoch > current_epoch {
                // It's still possible to see a rollback back to the previous epoch, but the worst thing
                // to happen is that we delete some unconfirmed TXs, which is fine.
                self.unconfirmed_harvest_tx_index.notify_end_of_epoch();
            }
        }
        trace!("Confirmed block slot: {}", block_slot);
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
    UHarvestIndex: UnconfirmedRewardTxIndex + Send + Sync,
    Prov: TxProver<SignedTxBuilder, Transaction> + Send + Sync,
    Ctx: Has<MinLovelacePerHarvest>
        + Has<DeployedScriptInfo<{ ProtocolValidator::HarvestOrder as u8 }>>
        + Has<DeployedScriptInfo<{ ProtocolValidator::PermManager as u8 }>>
        + Has<DeployedScriptInfo<{ ProtocolValidator::SmartFarm as u8 }>>
        + Has<DeployedScriptInfo<{ ProtocolValidator::BufferWallet as u8 }>>
        + Has<DeployedScriptInfo<{ ProtocolValidator::FarmFactory as u8 }>>
        + Has<DeployedScriptInfo<{ ProtocolValidator::MintWpAuthPolicy as u8 }>>
        + Has<FarmFactoryAuthPolicy>
        + Has<NetworkId>
        + Has<GenesisEpochStartTime>
        + Has<PermManagerAuthPolicy>
        + Has<SplashPolicy>
        + Has<OperatorCreds>
        + Has<AuthorizedExecutors>
        + Has<BufferWalletAuthPolicy>
        + Has<PrivateKey>
        + Sync,
{
    async fn try_approve(&mut self, tx: &TxCosignRequest, ctx: &Ctx) -> Option<Transaction> {
        let current_slot = self.get_current_slot()?;
        match tx {
            TxCosignRequest::Harvest(tx) => {
                // Check signature
                let signed_tx_builder = SignedTxBuilder::from(tx.tx.clone());
                let tx_hash = signed_tx_builder.clone().build_unchecked().canonical_hash();

                let valid_tx_signature = {
                    let vkeys = &signed_tx_builder.witness_set().vkeys;

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
                    buffer_wallet_withdrawn_amount: BufferWalletSplashTokenDecrease(bw_withdrawn_amount),
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
                        if order.issued_at.1 != current_epoch {
                            info!(
                                "Order issued_at epoch ({}) does not match current epoch ({}) in Harvest TX.",
                                order.issued_at.1, current_epoch
                            );
                            return None;
                        }
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

                    if total_payout != bw_withdrawn_amount {
                        info!(
                            "Total payout ({}) does not match buffer wallet withdrawn amount ({}) in Harvest TX.",
                            total_payout, bw_withdrawn_amount
                        );
                        return None;
                    }

                    let consumed_buffer_wallet_tx_hash = buffer_wallet_update.consumed?.tx_hash();
                    if self.unconfirmed_harvest_tx_index.try_add_tx(
                        consumed_buffer_wallet_tx_hash,
                        tx_hash,
                        payouts.iter().map(|(order, _)| order.account_key).collect(),
                    ) {
                        return Some(self.prover.prove(tx.tx.clone().into()));
                    }
                }
            }
            TxCosignRequest::GaugeBuffering(tx) => {
                let suspended_gauges = self.index.get_auth_manager().await?.0.suspended_gauges;

                let operator_pkh = ctx.select::<OperatorCreds>().0;
                trace!("verifier operator_pkh: {}", operator_pkh.to_hex());

                let signed_tx_builder: SignedTxBuilder = tx.tx.clone().into();
                // Check signature
                let tx_hash = signed_tx_builder.clone().build_unchecked().canonical_hash();

                let valid_tx_signature = {
                    let vkeys = &signed_tx_builder.witness_set().vkeys;

                    if vkeys.len() == 1 {
                        let Vkeywitness {
                            vkey,
                            ed25519_signature,
                            ..
                        } = vkeys.values().next().unwrap();

                        let authorized_signers = ctx.select::<AuthorizedExecutors>().0;
                        trace!("verifying vkey with tx hash: {}", tx_hash.to_hex());
                        let valid_signature = vkey.verify(tx_hash.to_raw_bytes(), ed25519_signature);
                        let is_authorized = authorized_signers.contains(vkey);
                        if !valid_signature || !is_authorized {
                            warn!(
                                "Invalid TX signature: valid_signature: {}, is_authorized: {}",
                                valid_signature, is_authorized
                            );
                        }
                        valid_signature && is_authorized
                    } else {
                        false
                    }
                };

                if !valid_tx_signature {
                    return None;
                }

                let tx_view = to_tx_view_partially_resolved(tx, current_slot);
                let parse_result = OnChainEvent::try_from_ledger(&tx_view, ctx);
                trace!("parse_result: {:?}", parse_result);
                if let Some(OnChainEvent::BotGaugeBufferingAction {
                    drained_gauges: GaugeWithdrawals(drained_gauges),
                    buffer_wallet_deposited_amount: BufferWalletSplashTokenIncrease(bw_deposited_amount),
                    ..
                }) = parse_result
                {
                    let mut total_rewards = 0;
                    for (gauge_update, BufferWalletSplashTokenDecrease(amount)) in &drained_gauges {
                        // Check none of the gauges are suspended
                        if suspended_gauges.contains(&gauge_update.created.0.id) {
                            return None;
                        }
                        total_rewards += *amount;
                    }

                    if total_rewards != bw_deposited_amount {
                        warn!(
                            "Total rewards ({}) do not match buffer wallet deposited amount ({}) in GaugeBuffering TX.",
                            total_rewards,
                            bw_deposited_amount
                        );
                        return None;
                    }
                    let mut ss_builder = tx.tx.clone();
                    let sk = ctx.select::<PrivateKey>();
                    let signature = make_vkey_witness(&tx_hash, &sk);
                    ss_builder.witness_set.add_vkey(signature);

                    let signed_tx_builder: SignedTxBuilder = ss_builder.into();
                    let tx = self.prover.prove(signed_tx_builder);
                    assert_eq!(tx.canonical_hash(), tx_hash);
                    return Some(tx);
                } else {
                    warn!("Couldn't parse OnChainEvent::BotGaugeBufferingAction");
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
    let signed_tx_builder: SignedTxBuilder = partially_signed_tx.tx.clone().into();
    let built_tx = signed_tx_builder.build_unchecked();
    let hash = built_tx.canonical_hash();
    let inputs: Vec<_> = partially_signed_tx
        .inputs
        .iter()
        .zip(built_tx.body.inputs)
        .map(|(cardano_tx_input, input)| {
            // TODO: address hard-coding of slot.
            let timed_output = Some(TimedOutput {
                output: cardano_tx_input.tx_output.clone(),
                slot: 1000,
            });
            //let timed_output = cardano_tx_input.issued_at.map(|slot| TimedOutput {
            //    output: cardano_tx_input.tx_output.clone(),
            //    slot: slot.0,
            //});

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

#[cfg(test)]
mod tests {
    use cml_chain::{builders::tx_builder::SignedTxBuilder, transaction::TransactionBody};
    use cml_crypto::RawBytesEncoding;
    use spectrum_cardano_lib::{hash::hash_transaction_canonical, AssetName, Token};
    use spectrum_offchain::tx_hash::CanonicalHash;
    use spectrum_offchain_cardano::data::PoolId;
    use splash_dao_offchain::deployment::IssuedAsset;

    use crate::engine::verifier::TxCosignRequest;

    #[test]
    fn encode_hex() {
        let hex0 = hex::encode(&[]);
        let hex1 = hex::encode(&[65, 68, 65, 95, 85, 83, 68, 77, 95, 78, 70, 84]);
        let hex2 = hex::encode(&[
            186, 204, 186, 164, 67, 247, 95, 24, 61, 92, 165, 154, 55, 193, 197, 225, 149, 67, 24, 63, 92,
            109, 138, 233, 239, 198, 208, 54,
        ]);
        println!("hex0: {}", hex0);
        println!("hex1: {}", hex1);
        println!("hex2: {}", hex2);
    }

    #[test]
    fn test_asset_name() {
        let json_input = r#"{
        "initial_farms": [
    {
      "policy_id": "baccbaa443f75f183d5ca59a37c1c5e19543183f5c6d8ae9efc6d036",
      "asset_name": "4144415f5553444d5f4e4654",
      "quantity": "1"
    },
    {
      "policy_id": "bce349fc159b2d715abba7e1de4eb35c0096cb28483f72b061ef7f8e",
      "asset_name": "4144415f744d494e5f4e4654",
      "quantity": "1"
    }
  ]
}"#;
        #[derive(serde::Deserialize)]
        struct Wrapper {
            initial_farms: Vec<IssuedAsset>,
        }

        let wrapper: Wrapper = serde_json::from_str(json_input).unwrap();
        let tokens = wrapper
            .initial_farms
            .into_iter()
            .map(|asset| {
                let asset_name = AssetName::from(asset.asset_name);
                PoolId(Token(asset.policy_id, asset_name))
            })
            .collect::<Vec<_>>();

        let pool_id_roundtrip: Vec<PoolId> = tokens
            .iter()
            .cloned()
            .map(|pool_id| {
                let mut bytes = pool_id.0 .0.to_raw_bytes().to_vec();
                bytes.extend(pool_id.0 .1.as_bytes());
                PoolId::try_from(bytes.as_slice()).unwrap()
            })
            .collect();
        assert_eq!(tokens, pool_id_roundtrip);
    }

    #[test]
    fn debug_payload() {
        let hex_payload = "81ae4761756765427566666572696e6782a2747884af626f64795f63626f725f6279746573dc044acca900ccd90102cc84cc8258205d0d19ccf1ccd678ccedcca7ccb857cc9c2bccefcce9cca1cc820329cc9fccef57ccfacceccc913acc9f0dccb259cc8fcccacc8e00cc8258207cccef444662cca30c2bcce6cce4cc8accb0ccd20accdfccfacc8449ccb42f2a1ecc9250ccf6ccbb27cc91627f626a01cc825820cca97c7dccebccd4ccaa147c18ccfa66ccd52bcce07b6135cca1ccfeccab33cc9b59ccb9cca0ccc1ccc85c455bccca7501cc825820ccf83447ccf66cccf563ccfbccec61cca1111eccb52c1d6acc9eccb5ccae5a27ccf6ccfa791b70ccb8cc84ccb3cc960b0601cc84cca300581d70ccdd6012cca60dcc8bcc82ccceccf26105ccefcc8cccf9ccf47ccc902d583d78ccbcccdcccb54720ccc4ccb501cc821a001973cc82cca2581c0accb7cc96cc8950ccc81accc94351ccbfccaacc9a2a7862773cccf05e2a68343acc9f3b49ccb8cca14653504c4153481b00000058cca972cce000581cccb8ccb9315accf3cca44bccbd53ccc0ccd4ccc5cc8ecc85ccb1cc94ccc404ccba56cc9accb2cc90ccdfcca0cca35451cca141cca40102cc8201ccd8185862ccd879cc8258200000000000000000000000000000000000000000000000000000000000000000cc82581c415f2b60ccaa3fcc9e7e4c7a6bcc935c6c15ccc4cc9f0acce9ccf610ccf70d2fccceccf35132581c52202b0acc9acca9797c6d22ccb2cc9d0bcccd2b27cc82ccaaccca47ccdb097acca9cc9428ccf45ccca300581d70cc89ccd9cc811fcc9ccc8cccb02e7505cc8bcc85cc8bcc82ccb8ccffccb66d5ccc8accbdccbbcce1ccdacccfccb3363101cc821a001b07cca6cca1581ccc89ccd9cc811fcc9ccc8cccb02e7505cc8bcc85cc8bcc82ccb8ccffccb66d5ccc8accbdccbbcce1ccdacccfccb33631cca141010102cc8201ccd818586dccd879cc83581cccf350ccbc54cc90782765cca16169ccac48ccdd0b57ccd4ccb4ccb8cc8d3bccefccd6ccb6ccb7cc8b324b581cccb8ccb9315accf3cca44bccbd53ccc0ccd4ccc5cc8ecc85ccb1cc94ccc404ccba56cc9accb2cc90ccdfcca0cca35451ccd879cc82581cccbccce349ccfc15cc9b2d715accbbcca7cce1ccde4eccb35c00cc96cccb28483f72ccb061ccef7fcc8e4c4144415f744d494e5f4e4654cca300581d70cc89ccd9cc811fcc9ccc8cccb02e7505cc8bcc85cc8bcc82ccb8ccffccb66d5ccc8accbdccbbcce1ccdacccfccb3363101cc821a001b07cca6cca1581ccc89ccd9cc811fcc9ccc8cccb02e7505cc8bcc85cc8bcc82ccb8ccffccb66d5ccc8accbdccbbcce1ccdacccfccb33631cca141000102cc8201ccd818586dccd879cc83581cccf350ccbc54cc90782765cca16169ccac48ccdd0b57ccd4ccb4ccb8cc8d3bccefccd6ccb6ccb7cc8b324b581cccb8ccb9315accf3cca44bccbd53ccc0ccd4ccc5cc8ecc85ccb1cc94ccc404ccba56cc9accb2cc90ccdfcca0cca35451ccd879cc82581cccbaccccccbacca443ccf75f183d5ccca5cc9a37ccc1ccc5cce1cc9543183f5c6dcc8acce9ccefccc6ccd0364c4144415f5553444d5f4e4654cc82583900415f2b60ccaa3fcc9e7e4c7a6bcc935c6c15ccc4cc9f0acce9ccf610ccf70d2fccceccf3513222cc95cc9bcc92ccb5cc98ccd74e751bccb13d4bccb320cc94cc9417ccceccb9cc814857cc85cc8bccb038cce21a3bcc8a69ccca021a000d59ccc2031a06ccbfccd9ccab081a06ccbfccd9cca10b582024ccc2cc8bccfb6779cca43accc425281e5d1d281e2300cc98ccfc0408cca566cc83cc84cce125ccf2cca5ccfaccfe0dccd90102cc81cc825820ccd7305b1e426f7f13cce3cca14034cc9c10ccb9ccb1cccaccadcc8f455c307173cccd3c05ccafccc6ccd3ccd7cce1000eccd90102cc82581c415f2b60ccaa3fcc9e7e4c7a6bcc935c6c15ccc4cc9f0acce9ccf610ccf70d2fccceccf35132581c52202b0acc9acca9797c6d22ccb2cc9d0bcccd2b27cc82ccaaccca47ccdb097acca9cc9428ccf45c12ccd90102cc83cc8258200acc9cccabcc881557164a4369ccc2ccc6cca8ccb5ccfaccd9ccc449ccb6cc8dccef28ccf814cca729cc9fcc8dcca42bccf8cc8302cc8258200acc9cccabcc881557164a4369ccc2ccc6cca8ccb5ccfaccd9ccc449ccb6cc8dccef28ccf814cca729cc9fcc8dcca42bccf8cc8307cc825820ccf83447ccf66cccf563ccfbccec61cca1111eccb52c1d6acc9eccb5ccae5a27ccf6ccfa791b70ccb8cc84ccb3cc960b05ab7769746e6573735f73657486a5766b65797381d945656432353531395f706b31766830656b75387635637838616e77643334646e38717a7637676e336c7268366165777a7067336763713070777970666178677333353939657282a4766b6579d945656432353531395f706b31766830656b75387635637838616e77643334646e38717a7637676e336c7268366165777a70673367637130707779706661786773333539396572b1656432353531395f7369676e6174757265d9806661393563373662343730393335323338316266643862663135393532616136373735323033633062356334343135386231303865326334396463306637656262366466306362333832316566616362353361336233363765363461303239306439303863656266623264376131646636636339326334666365333736393063aa626f6f7473747261707380a77363726970747380ab706c757475735f6461746180a972656465656d6572738382a3746167a55370656e64a5696e6465780184a3746167a55370656e64a5696e64657801a46461746182ab636f6e7374727563746f7281bc2473657264655f6a736f6e3a3a707269766174653a3a4e756d626572a131a66669656c64739182ab636f6e7374727563746f7281bc2473657264655f6a736f6e3a3a707269766174653a3a4e756d626572a130a66669656c64739281a3696e7481bc2473657264655f6a736f6e3a3a707269766174653a3a4e756d626572a13182ab636f6e7374727563746f7281bc2473657264655f6a736f6e3a3a707269766174653a3a4e756d626572a131a66669656c64739281a3696e7481bc2473657264655f6a736f6e3a3a707269766174653a3a4e756d626572a13181a3696e7481bc2473657264655f6a736f6e3a3a707269766174653a3a4e756d626572a133a865785f756e69747382a36d656dce0007a120a57374657073ce0bebc20082a3746167a55370656e64a5696e6465780284a3746167a55370656e64a5696e64657802a46461746182ab636f6e7374727563746f7281bc2473657264655f6a736f6e3a3a707269766174653a3a4e756d626572a131a66669656c64739182ab636f6e7374727563746f7281bc2473657264655f6a736f6e3a3a707269766174653a3a4e756d626572a130a66669656c64739281a3696e7481bc2473657264655f6a736f6e3a3a707269766174653a3a4e756d626572a13282ab636f6e7374727563746f7281bc2473657264655f6a736f6e3a3a707269766174653a3a4e756d626572a131a66669656c64739281a3696e7481bc2473657264655f6a736f6e3a3a707269766174653a3a4e756d626572a13181a3696e7481bc2473657264655f6a736f6e3a3a707269766174653a3a4e756d626572a133a865785f756e69747382a36d656dce0007a120a57374657073ce0bebc20082a3746167a55370656e64a5696e6465780384a3746167a55370656e64a5696e64657803a46461746182ab636f6e7374727563746f7281bc2473657264655f6a736f6e3a3a707269766174653a3a4e756d626572a130a66669656c647390a865785f756e69747382a36d656dce000f4240a57374657073ce05f5e100ad72657175697265645f7769747386a5766b65797392d9383431356632623630616133663965376534633761366239333563366331356334396630616539663631306637306432666365663335313332d9383532323032623061396161393739376336643232623239643062636432623237383261616361343764623039376161393934323866343563aa626f6f7473747261707390a77363726970747390ab706c757475735f6461746190a972656465656d65727390ab7363726970745f7265667392d9383839643938313166396338636230326537353035386238353862383262386666623636643563386162646262653164616366623333363331d9386464363031326136306438623832636566323631303565663863663966343763393032643538336437386263646362353437323063346235a869735f76616c6964c3ae617578696c696172795f64617461c0a6696e707574739483aa6f75747075745f726566d942356430643139663164363738656461376238353739633262656665396131383230333239396665663537666165633931336139663064623235393866636138652330a974785f6f757470757481b1416c6f6e7a6f466f726d617454784f757483a761646472657373d96c616464725f74657374317170713437326d7134676c65756c6a7630663465786872767a687a66377a68663763673077726630656d65347a76337a6a6b64653964766336613838327861333834396d786779356a7374756177767066707463747a617338723371776a77373535a6616d6f756e7482a4636f696ece3b9aca00aa6d756c7469617373657480aa646174756d5f68617368c0a96973737565645f6174c083aa6f75747075745f726566d942376365663434343636326133306332626536653438616230643230616466666138343439623432663261316539323530663662623237393136323766363236612331a974785f6f757470757481b1436f6e776179466f726d617454784f757484a761646472657373d93f616464725f7465737431777a79616e71676c6e6a787471746e34716b3963747a757a68726c6d766d32753332376d686377366537656e76766777676b6a3664a6616d6f756e7482a4636f696ece001b07a6aa6d756c7469617373657482d938306162373936383935306338316163393433353162666161396132613738363237373363663035653261363833343361396633623439623881ac353335303463343135333438cf0000002c54b97000d938383964393831316639633863623032653735303538623835386238326238666662363664356338616264626265316461636662333336333181a2303101ac646174756d5f6f7074696f6e81a5446174756d81a5646174756d82ab636f6e7374727563746f7281bc2473657264655f6a736f6e3a3a707269766174653a3a4e756d626572a130a66669656c64739381a56279746573d938663335306263353439303738323736356131363136396163343864643062353764346234623838643362656664366236623738623332346281a56279746573d938623862393331356166336134346262643533633064346335386538356231393463343034626135363961623239306466613061333534353182ab636f6e7374727563746f7281bc2473657264655f6a736f6e3a3a707269766174653a3a4e756d626572a130a66669656c64739281a56279746573d938626365333439666331353962326437313561626261376531646534656233356330303936636232383438336637326230363165663766386581a56279746573b8343134343431356637343464343934653566346534363534b07363726970745f7265666572656e6365c0a96973737565645f6174c083aa6f75747075745f726566d942613937633764656264346161313437633138666136366435326265303762363133356131666561623333396235396239613063316338356334353562636137352331a974785f6f757470757481b1436f6e776179466f726d617454784f757484a761646472657373d93f616464725f7465737431777a79616e71676c6e6a787471746e34716b3963747a757a68726c6d766d32753332376d686377366537656e76766777676b6a3664a6616d6f756e7482a4636f696ece001b07a6aa6d756c7469617373657482d938306162373936383935306338316163393433353162666161396132613738363237373363663035653261363833343361396633623439623881ac353335303463343135333438cf0000002c54b97000d938383964393831316639633863623032653735303538623835386238326238666662363664356338616264626265316461636662333336333181a2303001ac646174756d5f6f7074696f6e81a5446174756d81a5646174756d82ab636f6e7374727563746f7281bc2473657264655f6a736f6e3a3a707269766174653a3a4e756d626572a130a66669656c64739381a56279746573d938663335306263353439303738323736356131363136396163343864643062353764346234623838643362656664366236623738623332346281a56279746573d938623862393331356166336134346262643533633064346335386538356231393463343034626135363961623239306466613061333534353182ab636f6e7374727563746f7281bc2473657264655f6a736f6e3a3a707269766174653a3a4e756d626572a130a66669656c64739281a56279746573d938626163636261613434336637356631383364356361353961333763316335653139353433313833663563366438616539656663366430333681a56279746573b8343134343431356635353533343434643566346534363534b07363726970745f7265666572656e6365c0a96973737565645f6174c083aa6f75747075745f726566d942663833343437663636636635363366626563363161313131316562353263316436613965623561653561323766366661373931623730623838346233393630622336a974785f6f757470757481b1436f6e776179466f726d617454784f757484a761646472657373d93f616464725f74657374317772776b71793478706b3963396e686a76797a376c7238653733376671743263383475746568393467757376666467683678366d76a6616d6f756e7482a4636f696ece00166d0eaa6d756c7469617373657481d938623862393331356166336134346262643533633064346335386538356231393463343034626135363961623239306466613061333534353181a2613401ac646174756d5f6f7074696f6e81a5446174756d81a5646174756d82ab636f6e7374727563746f7281bc2473657264655f6a736f6e3a3a707269766174653a3a4e756d626572a130a66669656c64739281a56279746573d9403030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303081a46c6973749281a56279746573d938343135663262363061613366396537653463376136623933356336633135633439663061653966363130663730643266636566333531333281a56279746573d9383532323032623061396161393739376336643232623239643062636432623237383261616361343764623039376161393934323866343563b07363726970745f7265666572656e6365c0a96973737565645f6174c0";
        let bytes = hex::decode(hex_payload).unwrap();
        let cosign_request: TxCosignRequest = rmp_serde::from_slice(&bytes).unwrap();
        let gauge_buffering = match cosign_request {
            TxCosignRequest::GaugeBuffering(tx) => tx,
            _ => panic!("Expected GaugeBuffering request"),
        };
        let signed_tx_builder = SignedTxBuilder::from(gauge_buffering.tx.clone());
        let tx_body = signed_tx_builder.clone().build_unchecked().body;
        println!("tx_hash: {:?}", tx_body.hash().to_hex());
        // dbg!(tx_body);
        // dbg!(gauge_buffering.inputs.last());
        // dbg!(tx_body.outputs.first());
        //let other_tx_hash = gauge_buffering
        //    .tx
        //    .build_checked()
        //    .unwrap()
        //    .canonical_hash()
        //    .to_hex();
        //println!("other_hash: {}", other_tx_hash);

        //let tx_body_json = r#"{"inputs":[{"transaction_id":"6a2090e4ee8ab9704887edb32771b3e9ae41aab47a1cd5d0b7ddd1ce04f4e2dc","index":1},{"transaction_id":"717aead1d19783b567de5cf356af2f74e017a0061833778e3d816c3620438879","index":0},{"transaction_id":"a37e7131f5b366497729207480311f4c5785ca3967c024bfb54cfccaf64d47a9","index":1},{"transaction_id":"b4183c189ade437fbe1e4b33ec1d99afa3d48ef52511d704444c90a6177476da","index":6}],"outputs":[{"ConwayFormatTxOut":{"address":"addr_test1wrs9cckdy3pfc73clk943x3q55hhh5zp90enerjvwy24nzsse3qnq","amount":{"coin":1667970,"multiasset":{"839772179bf83aa31cf57dc2230a49fe5331ec2885f7700864ef05e7":{"a4":1},"8bbb5af3cc08d0a573d5638704c4b25055f2d7d9d07034a9db568efc":{"53504c415348":744897453510}}},"datum_option":{"Datum":{"datum":{"constructor":0,"fields":[{"bytes":"0000000000000000000000000000000000000000000000000000000000000000"},{"list":[{"bytes":"415f2b60aa3f9e7e4c7a6b935c6c15c49f0ae9f610f70d2fcef35132"},{"bytes":"52202b0a9aa9797c6d22b29d0bcd2b2782aaca47db097aa99428f45c"}]}]}}},"script_reference":null}},{"ConwayFormatTxOut":{"address":"addr_test1wq4lxcv2rr7xxqgptt6u2e4nakxdesr4mpgnf3489p9a3rq3ex09t","amount":{"coin":1732640,"multiasset":{"2bf3618a18fc6301015af5c566b3ed8cdcc075d85134c6a7284bd88c":{"00":1}}},"datum_option":{"Datum":{"datum":{"constructor":0,"fields":[{"bytes":"1fa288812884e7e14f2d397b4acbb79e519eb4fe7d04d695c3e1153d"},{"constructor":0,"fields":[{"bytes":"d8eb52caf3289a2880288b23141ce3d2a7025dcf76f26fd5659add06"},{"bytes":"f1647220c6652c55f46e8581136d10e7e9569511ae3a050fd797defd15be757e"}]}]}}},"script_reference":null}},{"ConwayFormatTxOut":{"address":"addr_test1wq4lxcv2rr7xxqgptt6u2e4nakxdesr4mpgnf3489p9a3rq3ex09t","amount":{"coin":1732640,"multiasset":{"2bf3618a18fc6301015af5c566b3ed8cdcc075d85134c6a7284bd88c":{"01":1}}},"datum_option":{"Datum":{"datum":{"constructor":0,"fields":[{"bytes":"1fa288812884e7e14f2d397b4acbb79e519eb4fe7d04d695c3e1153d"},{"constructor":0,"fields":[{"bytes":"baccbaa443f75f183d5ca59a37c1c5e19543183f5c6d8ae9efc6d036"},{"bytes":"4144415f5553444d5f4e46540000000000000000000000000000000000000000"}]}]}}},"script_reference":null}},{"AlonzoFormatTxOut":{"address":"addr_test1qpq472mq4gleuljv0f4exhrvzhzf7zhf7cg0wrf0eme4zv3zjkde9dvc6a882xa3849mxgy5jstuawvpfptctzas8r3qwjw755","amount":{"coin":3940733,"multiasset":{}},"datum_hash":null}}],"fee":861007,"ttl":111848995,"certs":null,"withdrawals":null,"auxiliary_data_hash":null,"validity_interval_start":111848985,"mint":null,"script_data_hash":"a1fabd532e3b394782fc0649a1f4383ec9dab57e08960cb3e5c63608672da298","collateral_inputs":[{"transaction_id":"d7305b1e426f7f13e3a140349c10b9b1caad8f455c307173cd3c05afc6d3d7e1","index":0}],"required_signers":null,"network_id":null,"collateral_return":null,"total_collateral":null,"reference_inputs":[{"transaction_id":"76554bd2d3cc09868d94ace4f1fe8396fb1c7fe0eb6a91972f01bc5eeec79efd","index":2},{"transaction_id":"76554bd2d3cc09868d94ace4f1fe8396fb1c7fe0eb6a91972f01bc5eeec79efd","index":7},{"transaction_id":"b4183c189ade437fbe1e4b33ec1d99afa3d48ef52511d704444c90a6177476da","index":5}],"voting_procedures":null,"proposal_procedures":null,"current_treasury_value":null,"donation":null}"#;
        //let other_tx_body: TransactionBody = serde_json::from_str(tx_body_json).unwrap();
        //println!(
        //    "AAAAA: {}",
        //    hash_transaction_canonical(&other_tx_body.clone()).to_hex()
        //);
        // dbg!(tx_body);
        // dbg!(other_tx_body);
    }
}
