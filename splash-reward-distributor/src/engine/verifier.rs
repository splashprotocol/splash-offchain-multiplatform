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
        let hex_payload = "81ae4761756765427566666572696e6782a2747884af626f64795f63626f725f6279746573dc03f7cca800ccd90102cc84cc8258206a20cc90cce4cceecc8accb97048cc87ccedccb32771ccb3cce9ccae41ccaaccb47a1cccd5ccd0ccb7ccddccd1ccce04ccf4cce2ccdc01cc825820717acceaccd1ccd1cc97cc83ccb567ccde5cccf356ccaf2f74cce017cca006183377cc8e3dcc816c362043cc887900cc825820cca37e7131ccf5ccb3664977292074cc80311f4c57cc85ccca3967ccc024ccbfccb54cccfccccaccf64d47cca901cc825820ccb4183c18cc9accde437fccbe1e4b33ccec1dcc99ccafcca3ccd4cc8eccf52511ccd704444ccc90cca6177476ccda0601cc84cca300581d70cce05c62cccd2442cc9c7a38ccfdcc8b58cc9a20cca52f7bccd0412bccf33ccc8e4c711559cc8a01cc821a001973cc82cca2581ccc83cc977217cc9bccf83acca31cccf57dccc2230a49ccfe5331ccec28cc85ccf7700864ccef05cce7cca141cca401581ccc8bccbb5accf3cccc08ccd0cca573ccd563cc8704ccc4ccb25055ccf2ccd7ccd9ccd07034cca9ccdb56cc8eccfccca14653504c4153481b000000ccad6f591dccc602cc8201ccd8185862ccd879cc8258200000000000000000000000000000000000000000000000000000000000000000cc82581c415f2b60ccaa3fcc9e7e4c7a6bcc935c6c15ccc4cc9f0acce9ccf610ccf70d2fccceccf35132581c52202b0acc9acca9797c6d22ccb2cc9d0bcccd2b27cc82ccaaccca47ccdb097acca9cc9428ccf45ccca300581d702bccf361cc8a18ccfc6301015accf5ccc566ccb3ccedcc8cccdcccc075ccd85134ccc6cca7284bccd8cc8c01cc821a001a7020cca1581c2bccf361cc8a18ccfc6301015accf5ccc566ccb3ccedcc8cccdcccc075ccd85134ccc6cca7284bccd8cc8ccca141000102cc8201ccd8185864ccd879cc82581c1fcca2cc88cc8128cc84cce7cce14f2d397b4acccbccb7cc9e51cc9eccb4ccfe7d04ccd6cc95ccc3cce1153dccd879cc82581cccd8cceb52cccaccf328cc9a28cc8028cc8b23141ccce3ccd2cca7025dcccf76ccf26fccd565cc9accdd065820ccf1647220ccc6652c55ccf46ecc85cc81136d10cce7cce956cc9511ccae3a050fccd7cc97ccdeccfd15ccbe757ecca300581d702bccf361cc8a18ccfc6301015accf5ccc566ccb3ccedcc8cccdcccc075ccd85134ccc6cca7284bccd8cc8c01cc821a001a7020cca1581c2bccf361cc8a18ccfc6301015accf5ccc566ccb3ccedcc8cccdcccc075ccd85134ccc6cca7284bccd8cc8ccca141010102cc8201ccd8185864ccd879cc82581c1fcca2cc88cc8128cc84cce7cce14f2d397b4acccbccb7cc9e51cc9eccb4ccfe7d04ccd6cc95ccc3cce1153dccd879cc82581cccbaccccccbacca443ccf75f183d5ccca5cc9a37ccc1ccc5cce1cc9543183f5c6dcc8acce9ccefccc6ccd03658204144415f5553444d5f4e46540000000000000000000000000000000000000000cc82583900415f2b60ccaa3fcc9e7e4c7a6bcc935c6c15ccc4cc9f0acce9ccf610ccf70d2fccceccf3513222cc95cc9bcc92ccb5cc98ccd74e751bccb13d4bccb320cc94cc9417ccceccb9cc814857cc85cc8bccb038cce21a003c217d021a000d234f031a06ccabccba23081a06ccabccba190b5820cca1ccfaccbd532e3b3947cc82ccfc0649cca1ccf4383eccc9ccdaccb57e08cc960cccb3cce5ccc63608672dcca2cc980dccd90102cc81cc825820ccd7305b1e426f7f13cce3cca14034cc9c10ccb9ccb1cccaccadcc8f455c307173cccd3c05ccafccc6ccd3ccd7cce10012ccd90102cc83cc82582076554bccd2ccd3cccc09cc86cc8dcc94ccaccce4ccf1ccfecc83cc96ccfb1c7fcce0cceb6acc91cc972f01ccbc5ecceeccc7cc9eccfd02cc82582076554bccd2ccd3cccc09cc86cc8dcc94ccaccce4ccf1ccfecc83cc96ccfb1c7fcce0cceb6acc91cc972f01ccbc5ecceeccc7cc9eccfd07cc825820ccb4183c18cc9accde437fccbe1e4b33ccec1dcc99ccafcca3ccd4cc8eccf52511ccd704444ccc90cca6177476ccda05ab7769746e6573735f73657486a5766b65797381d945656432353531395f706b31766830656b75387635637838616e77643334646e38717a7637676e336c7268366165777a7067336763713070777970666178677333353939657282a4766b6579d945656432353531395f706b31766830656b75387635637838616e77643334646e38717a7637676e336c7268366165777a70673367637130707779706661786773333539396572b1656432353531395f7369676e6174757265d9803331343163616533326632653839623763373533633739303165306535383365633665313935313266653066623833663236366538373661643062323237633232333230376337376663623364376237613266303538626331356534646164393434313964333766303433633966356665323338666636303864646538663066aa626f6f7473747261707380a77363726970747380ab706c757475735f6461746180a972656465656d6572738382a3746167a55370656e64a5696e6465780084a3746167a55370656e64a5696e64657800a46461746182ab636f6e7374727563746f7281bc2473657264655f6a736f6e3a3a707269766174653a3a4e756d626572a131a66669656c64739182ab636f6e7374727563746f7281bc2473657264655f6a736f6e3a3a707269766174653a3a4e756d626572a130a66669656c64739281a3696e7481bc2473657264655f6a736f6e3a3a707269766174653a3a4e756d626572a13182ab636f6e7374727563746f7281bc2473657264655f6a736f6e3a3a707269766174653a3a4e756d626572a131a66669656c64739181a3696e7481bc2473657264655f6a736f6e3a3a707269766174653a3a4e756d626572a131a865785f756e69747382a36d656dce0007a120a57374657073ce0bebc20082a3746167a55370656e64a5696e6465780284a3746167a55370656e64a5696e64657802a46461746182ab636f6e7374727563746f7281bc2473657264655f6a736f6e3a3a707269766174653a3a4e756d626572a131a66669656c64739182ab636f6e7374727563746f7281bc2473657264655f6a736f6e3a3a707269766174653a3a4e756d626572a130a66669656c64739281a3696e7481bc2473657264655f6a736f6e3a3a707269766174653a3a4e756d626572a13282ab636f6e7374727563746f7281bc2473657264655f6a736f6e3a3a707269766174653a3a4e756d626572a131a66669656c64739181a3696e7481bc2473657264655f6a736f6e3a3a707269766174653a3a4e756d626572a131a865785f756e69747382a36d656dce0007a120a57374657073ce0bebc20082a3746167a55370656e64a5696e6465780384a3746167a55370656e64a5696e64657803a46461746182ab636f6e7374727563746f7281bc2473657264655f6a736f6e3a3a707269766174653a3a4e756d626572a130a66669656c647390a865785f756e69747382a36d656dce000f4240a57374657073ce00989680ad72657175697265645f7769747386a5766b65797391d9383431356632623630616133663965376534633761366239333563366331356334396630616539663631306637306432666365663335313332aa626f6f7473747261707390a77363726970747390ab706c757475735f6461746190a972656465656d65727390ab7363726970745f7265667392d9383262663336313861313866633633303130313561663563353636623365643863646363303735643835313334633661373238346264383863d9386530356336326364323434323963376133386664386235383961323061353266376264303431326266333363386534633731313535393861a869735f76616c6964c3ae617578696c696172795f64617461c0a6696e707574739483aa6f75747075745f726566d942366132303930653465653861623937303438383765646233323737316233653961653431616162343761316364356430623764646431636530346634653264632331a974785f6f757470757481b1436f6e776179466f726d617454784f757484a761646472657373d93f616464725f74657374317771346c786376327272377878716770747436753265346e616b7864657372346d70676e6633343839703961337271336578303974a6616d6f756e7482a4636f696ece001a7020aa6d756c7469617373657482d938326266333631386131386663363330313031356166356335363662336564386364636330373564383531333463366137323834626438386381a2303001d938386262623561663363633038643061353733643536333837303463346232353035356632643764396430373033346139646235363865666381ac353335303463343135333438cf00000056b7ac8ee3ac646174756d5f6f7074696f6e81a5446174756d81a5646174756d82ab636f6e7374727563746f7281bc2473657264655f6a736f6e3a3a707269766174653a3a4e756d626572a130a66669656c64739281a56279746573d938316661323838383132383834653765313466326433393762346163626237396535313965623466653764303464363935633365313135336482ab636f6e7374727563746f7281bc2473657264655f6a736f6e3a3a707269766174653a3a4e756d626572a130a66669656c64739281a56279746573d938643865623532636166333238396132383830323838623233313431636533643261373032356463663736663236666435363539616464303681a56279746573d94066313634373232306336363532633535663436653835383131333664313065376539353639353131616533613035306664373937646566643135626537353765b07363726970745f7265666572656e6365c0a96973737565645f6174c083aa6f75747075745f726566d942373137616561643164313937383362353637646535636633353661663266373465303137613030363138333337373865336438313663333632303433383837392330a974785f6f757470757481b1416c6f6e7a6f466f726d617454784f757483a761646472657373d96c616464725f74657374317170713437326d7134676c65756c6a7630663465786872767a687a66377a68663763673077726630656d65347a76337a6a6b64653964766336613838327861333834396d786779356a7374756177767066707463747a617338723371776a77373535a6616d6f756e7482a4636f696ece004c4b40aa6d756c7469617373657480aa646174756d5f68617368c0a96973737565645f6174c083aa6f75747075745f726566d942613337653731333166356233363634393737323932303734383033313166346335373835636133393637633032346266623534636663636166363464343761392331a974785f6f757470757481b1436f6e776179466f726d617454784f757484a761646472657373d93f616464725f74657374317771346c786376327272377878716770747436753265346e616b7864657372346d70676e6633343839703961337271336578303974a6616d6f756e7482a4636f696ece001a7020aa6d756c7469617373657482d938326266333631386131386663363330313031356166356335363662336564386364636330373564383531333463366137323834626438386381a2303101d938386262623561663363633038643061353733643536333837303463346232353035356632643764396430373033346139646235363865666381ac353335303463343135333438cf00000056b7ac8ee3ac646174756d5f6f7074696f6e81a5446174756d81a5646174756d82ab636f6e7374727563746f7281bc2473657264655f6a736f6e3a3a707269766174653a3a4e756d626572a130a66669656c64739281a56279746573d938316661323838383132383834653765313466326433393762346163626237396535313965623466653764303464363935633365313135336482ab636f6e7374727563746f7281bc2473657264655f6a736f6e3a3a707269766174653a3a4e756d626572a130a66669656c64739281a56279746573d938626163636261613434336637356631383364356361353961333763316335653139353433313833663563366438616539656663366430333681a56279746573d94034313434343135663535353334343464356634653436353430303030303030303030303030303030303030303030303030303030303030303030303030303030b07363726970745f7265666572656e6365c0a96973737565645f6174c083aa6f75747075745f726566d942623431383363313839616465343337666265316534623333656331643939616661336434386566353235313164373034343434633930613631373734373664612336a974785f6f757470757481b1436f6e776179466f726d617454784f757484a761646472657373d93f616464725f74657374317772733963636b6479337066633733636c6b3934337833713535686868357a703930656e65726a76777932346e7a73736533716e71a6616d6f756e7482a4636f696ece00166d0eaa6d756c7469617373657481d938383339373732313739626638336161333163663537646332323330613439666535333331656332383835663737303038363465663035653781a2613401ac646174756d5f6f7074696f6e81a5446174756d81a5646174756d82ab636f6e7374727563746f7281bc2473657264655f6a736f6e3a3a707269766174653a3a4e756d626572a130a66669656c64739281a56279746573d9403030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303030303081a46c6973749281a56279746573d938343135663262363061613366396537653463376136623933356336633135633439663061653966363130663730643266636566333531333281a56279746573d9383532323032623061396161393739376336643232623239643062636432623237383261616361343764623039376161393934323866343563b07363726970745f7265666572656e6365c0a96973737565645f6174c0";
        let bytes = hex::decode(hex_payload).unwrap();
        let cosign_request: TxCosignRequest = rmp_serde::from_slice(&bytes).unwrap();
        let gauge_buffering = match cosign_request {
            TxCosignRequest::GaugeBuffering(tx) => tx,
            _ => panic!("Expected GaugeBuffering request"),
        };
        let signed_tx_builder = SignedTxBuilder::from(gauge_buffering.tx.clone());
        let tx_body = signed_tx_builder.clone().build_checked().unwrap().body;
        // dbg!(tx_body);
        // dbg!(gauge_buffering.inputs.last());
        dbg!(tx_body.outputs.first());
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
