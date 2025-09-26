use crate::accounts::{AccountReward, Accounts, LockedByAnotherReq};
use crate::constants::{
    GAUGE_BUFFERING_TX_FEE_DELTA, GAUGE_BUFFERING_TX_MINIMAL_FUNDING_BOX_BALANCE,
    HARVESTING_TX_ASSUMED_BASE_FEE, HARVESTING_TX_FEE_DELTA,
};
use crate::engine::batch::{BufferingBatch, HarvestBatch, OrderWithSpendDetails};
use crate::engine::resolved_tx::{
    CardanoTxInput, CardanoTxInputs, PartiallySignedCardanoTx, PartiallySignedTx,
};
use crate::engine::task::{GaugeBuffering, Harvesting, Task, TaskId};
use crate::engine::verifier::{RemoteVerifier, VerifierRejection};
use crate::entity_index::{AuthManagerIndex, HarvestOrderIndex, HarvestOrderSpend};
use crate::entity_index::{BufferWalletIndex, GaugeIndex};
use async_trait::async_trait;
use bloom_offchain::execution_engine::bundled::Bundled;
use cml_chain::builders::input_builder::{InputBuilderResult, SingleInputBuilder};
use cml_chain::builders::output_builder::{SingleOutputBuilderResult, TransactionOutputBuilder};
use cml_chain::builders::tx_builder::{ChangeSelectionAlgo, TransactionUnspentOutput};
use cml_chain::builders::witness_builder::{
    NativeScriptWitnessInfo, PartialPlutusWitness, PlutusScriptWitness,
};
use cml_chain::certs::Credential;
use cml_chain::transaction::{Transaction, TransactionInput};
use cml_chain::{RequiredSigners, Value};
use cml_crypto::{RawBytesEncoding, TransactionHash};
use log::{error, warn};
use pallas_network::miniprotocols::localtxsubmission::cardano_node_errors::{
    ApplyTxError, ConwayLedgerPredFailure, ConwayUtxoPredFailure, ConwayUtxowPredFailure, TxInput,
};
use rs_merkle::algorithms::Keccak256;
use rs_merkle::MerkleTree;
use serde::de::DeserializeOwned;
use serde::Serialize;
use spectrum_cardano_lib::collateral::Collateral;
use spectrum_cardano_lib::hash::hash_transaction_canonical;
use spectrum_cardano_lib::output::FinalizedTxOut;
use spectrum_cardano_lib::plutus_data::IntoPlutusData;
use spectrum_cardano_lib::transaction::TransactionOutputExtension;
use spectrum_cardano_lib::value::ValueExtension;
use spectrum_cardano_lib::{AssetClass, AssetName, NetworkId, OutputRef, Token};
use spectrum_offchain::domain::event::Predicted;
use spectrum_offchain::domain::Has;
use spectrum_offchain::network::Network;
use spectrum_offchain::tx_hash::CanonicalHash;
use spectrum_offchain_cardano::deployment::DeployedValidator;
use spectrum_offchain_cardano::tx_submission::RejectReasons;
use splash_dao_offchain::constants::SPLASH_NAME;
use splash_dao_offchain::deployment::{DaoScriptData, ProtocolValidator::*};
use splash_dao_offchain::entities::onchain::funding_box::{FundingBox, FundingBoxId};
use splash_dao_offchain::entities::onchain::smart_farm::{self, FarmId};
use splash_dao_offchain::funding::{AvailableFundingBoxes, FundingRepo};
use splash_dao_offchain::protocol_config::{BufferWalletScript, OperatorCreds, SplashPolicy};
use splash_dao_offchain::routines::actions::{BlueprintEstimates, DaoTxBlueprint};
use splash_dao_offchain::routines::{slot_to_epoch, time_millis_to_epoch, FundingBoxChanges, Slot};
use splash_dao_offchain::GenesisEpochStartTime;
use splash_yf_offchain::entities::buffer_wallet::{BufferWallet, BufferWalletWrap};
use splash_yf_offchain::entities::gauge::Gauge;
use splash_yf_offchain::entities::harvest_order::{HarvestOrder, HarvestOrderAction};
use splash_yf_offchain::events::EntityUpdated;
use splash_yf_offchain::Epoch;
use std::fmt::Display;
use std::hash::Hash;
use std::marker::PhantomData;
use std::time::SystemTime;

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Control<TaskId> {
    /// Task should be dropped
    Drop(TaskId),
    /// Ready to accept at least one more task
    Next,
    /// Batch is full
    Stop,
}

#[derive(Debug)]
pub struct ExecutionResult<TaskId, Out> {
    pub executed_tasks: Vec<TaskId>,
    pub output: Out,
}

pub struct HarvestFlowEntityUpdates<StateId, Bearer, Tx, TxInputs> {
    pub predicted_buffer_wallet_update: EntityUpdated<BufferWallet<StateId>, StateId, Bearer>,
    pub predicted_harvest_order_spends: Vec<(StateId, HarvestOrderSpend)>,
    pub predicted_merkle_tree: MerkleTree<Keccak256>,
    pub resolved_tx: PartiallySignedTx<Tx, TxInputs>,
}

pub struct BufferingFlowEntityUpdates<StateId, GaugeId, Bearer, Tx, TxInputs> {
    pub predicted_gauge_updates: Vec<EntityUpdated<Gauge<GaugeId, StateId>, StateId, Bearer>>,
    pub predicted_buffer_wallet_update: EntityUpdated<BufferWallet<StateId>, StateId, Bearer>,
    pub funding_box_changes: Option<FundingBoxChanges>,
    pub resolved_tx: PartiallySignedTx<Tx, TxInputs>,
}

#[async_trait]
pub trait BatchExecutor<TaskId, Task, Out, Err> {
    async fn feed(&mut self, task_id: TaskId, task: Task) -> Control<TaskId>;
    async fn execute(&mut self) -> Result<ExecutionResult<TaskId, Out>, Err>;
}

#[derive(Debug, Clone)]
pub enum Error {
    TxInputsAlreadySpent { failed_task_ids: Vec<TaskId> },
    UnrecoverableNodeError,
}

#[derive(Clone)]
pub struct HarvestingFlow<StateId, Bearer, Ctx, PositionIndex, OnChainIndex> {
    position_index: PositionIndex,
    onchain_index: OnChainIndex,
    batch: Option<HarvestBatch<StateId, Bearer>>,
    ctx: Ctx,
}

#[async_trait]
impl<Ctx, PositionIndex, OnChainIndex>
    BatchExecutor<
        TaskId,
        Harvesting<OutputRef>,
        HarvestFlowEntityUpdates<OutputRef, FinalizedTxOut, Transaction, CardanoTxInputs>,
        Error,
    > for HarvestingFlow<OutputRef, FinalizedTxOut, Ctx, PositionIndex, OnChainIndex>
where
    PositionIndex: Accounts<OutputRef> + Send,
    OnChainIndex:
        BufferWalletIndex<OutputRef, FinalizedTxOut> + HarvestOrderIndex<OutputRef, FinalizedTxOut> + Send,
    Ctx: Send
        + Has<BufferWalletScript>
        + Has<DeployedValidator<{ HarvestOrder as u8 }>>
        + Has<SplashPolicy>
        + Has<OperatorCreds>
        + Has<Collateral>
        + Has<GenesisEpochStartTime>
        + Has<NetworkId>,
{
    async fn feed(&mut self, task_id: TaskId, task: Harvesting<OutputRef>) -> Control<TaskId> {
        let order = if let Some(order) = self
            .onchain_index
            .read_designated_harvest_order(task.order_id)
            .await
        {
            order
        } else {
            return Control::Drop(task_id);
        };
        let batch = if let Some(ref mut batch) = self.batch {
            batch
        } else if let Some(Bundled(wallet_wrap, tx_out)) = self.onchain_index.get_buffer_wallet().await {
            let bundled = Bundled(wallet_wrap.wallet, tx_out);
            let input_merkle_tree = if let Some(m) = wallet_wrap.predicted_merkle_tree {
                m
            } else {
                self.onchain_index
                    .last_confirmed_merkle_tree()
                    .await
                    .unwrap()
                    .tree
            };
            self.batch.insert(HarvestBatch::new(bundled, input_merkle_tree))
        } else {
            error!("No buffer wallet found");
            return Control::Stop;
        };
        let (req, tx_out) = match order {
            crate::entity_index::Mod::Confirmed(Bundled(req, tx_out))
            | crate::entity_index::Mod::Predicted(Bundled(req, tx_out)) => (req.0, tx_out),
        };

        let now = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;
        let genesis_start_time = self.ctx.select::<GenesisEpochStartTime>();
        let current_epoch = Epoch::from(time_millis_to_epoch(now, genesis_start_time).0 as u64);
        let epoch_start = self
            .onchain_index
            .last_epoch_harvested(req.account_key)
            .await
            .map(|e| e.next())
            .unwrap_or(Epoch::from(0));

        match self
            .position_index
            .query_account_reward(&Credential::new_pub_key(req.account_key), epoch_start)
            .await
        {
            Some(AccountReward {
                accumulated_amount: amount,
                latest_epoch_inclusive,
            }) => {
                if latest_epoch_inclusive.next() < current_epoch {
                    warn!("lp-indexer chain-index lags reward-bot");
                    return Control::Stop;
                }
                let network_id = self.ctx.select::<NetworkId>();
                let spend = HarvestOrderSpend {
                    amount,
                    epoch_start,
                    epoch_end: latest_epoch_inclusive,
                    user: req.account_key,
                    reward_address: req.reward_receiver.to_address(network_id),
                };
                let order_id = req.id;
                let order_bundle = Bundled(req, tx_out);
                let order = OrderWithSpendDetails { order_bundle, spend };
                if batch.can_accept(amount) {
                    batch.add_order(order);
                } else {
                    warn!(
                        "Buffer wallet is running out of funds, cannot process request {}",
                        order_id
                    );
                    return Control::Stop;
                }
            }
            None => {
                warn!(
                    "Account {} not found, dropping request {}",
                    hex::encode(req.account_key.to_raw_bytes()),
                    req.id
                );
                return Control::Drop(task_id);
            }
        }
        Control::Next
    }

    async fn execute(
        &mut self,
    ) -> Result<
        ExecutionResult<
            TaskId,
            HarvestFlowEntityUpdates<OutputRef, FinalizedTxOut, Transaction, CardanoTxInputs>,
        >,
        Error,
    > {
        if let Some(batch) = self.batch.take() {
            let harvest_order_deployed_validator =
                self.ctx.select::<DeployedValidator<{ HarvestOrder as u8 }>>();
            let harvest_order_ref_script_output = harvest_order_deployed_validator.reference_utxo;

            let num_payouts = batch.orders.len() as u64;

            let buffer_wallet_script = self.ctx.select::<BufferWalletScript>().0;

            let mut accounts = vec![];

            let harvest_order_redeemer = HarvestOrderAction::Harvest.into_pd();
            let harvest_order_script_hash = harvest_order_deployed_validator.hash;
            let harvest_order_witness = PartialPlutusWitness::new(
                PlutusScriptWitness::Ref(harvest_order_script_hash),
                harvest_order_redeemer,
            );
            let ex_units = Some(DaoScriptData::global().harvest_order.ex_units.clone());
            let network_id = self.ctx.select::<NetworkId>();
            let mut predicted_harvest_order_spends = vec![];

            struct InputData {
                input: InputBuilderResult,
                ex_units: Option<cml_chain::plutus::ExUnits>,
                issued_at: Option<(Slot, Epoch)>,
            }

            let mut sorted_input_data: Vec<_> = batch
                .orders
                .into_iter()
                .map(
                    |OrderWithSpendDetails {
                         order_bundle:
                             Bundled(
                        HarvestOrder {
                            reward_receiver,
                            issued_at,
                            ..
                        },
                        tx_out,
                    ),
                         spend,
                     }| {
                        let reward_receiver = reward_receiver.to_address(network_id);
                        accounts.push((reward_receiver, spend.amount, tx_out.0.value().coin));
                        let harvest_order_input =
                            SingleInputBuilder::new(TransactionInput::from(tx_out.1), tx_out.0)
                                .plutus_script_inline_datum(
                                    harvest_order_witness.clone(),
                                    RequiredSigners::from(vec![]),
                                )
                                .unwrap();

                        predicted_harvest_order_spends.push((tx_out.1, spend));

                        InputData {
                            input: harvest_order_input,
                            ex_units: ex_units.clone(),
                            issued_at: Some(issued_at),
                        }
                    },
                )
                .collect();

            let Bundled(buffer_wallet_in, FinalizedTxOut(bw_tx_out, bw_in_output_ref)) = batch.buffer_wallet;

            let buffer_wallet_input =
                SingleInputBuilder::new(TransactionInput::from(bw_in_output_ref), bw_tx_out.clone())
                    .native_script(
                        buffer_wallet_script.clone(),
                        NativeScriptWitnessInfo::num_signatures(2),
                    )
                    .unwrap();
            sorted_input_data.push(InputData {
                input: buffer_wallet_input,
                ex_units: None,
                issued_at: None,
            });

            sorted_input_data.sort_by_key(|InputData { input, .. }| input.input.clone());

            let blueprint_sorted_inputs: Vec<_> = sorted_input_data
                .iter()
                .map(|InputData { input, ex_units, .. }| (input.clone(), ex_units.clone()))
                .collect();

            // Outputs

            let mut bw_out = bw_tx_out;

            let splash_asset_name = AssetName::from_utf8(SPLASH_NAME.into());
            let splash_policy = self.ctx.select::<SplashPolicy>().0;
            let splash_asset_class = AssetClass::Token(Token(splash_policy, splash_asset_name));

            assert!(bw_out
                .value_mut()
                .checked_sub(&make_splash_value(splash_asset_class, batch.total_payout))
                .is_ok());

            let buffer_wallet_output = SingleOutputBuilderResult::new(bw_out.clone());

            let mut outputs = vec![buffer_wallet_output];

            for (reward_receiver, payout, coin) in accounts {
                let mut user_value = Value::from(coin);
                // The TX fee is shared equally among all accounts receiving a payout.
                let reduction = HARVESTING_TX_ASSUMED_BASE_FEE / num_payouts;
                assert!(user_value.coin > reduction);
                user_value.coin -= reduction;
                user_value.add_unsafe(splash_asset_class, payout);

                let user_payout_output = TransactionOutputBuilder::new()
                    .with_address(reward_receiver)
                    .next()
                    .unwrap()
                    .with_value(user_value)
                    .build()
                    .unwrap();
                outputs.push(user_payout_output);
            }

            // Use blueprint to determine the total change amount. Then evenly distribute among all
            // receivers of payout.
            let OperatorCreds(_, operator_address) = self.ctx.select::<OperatorCreds>();
            let mut blueprint = DaoTxBlueprint {
                reference_inputs: vec![harvest_order_ref_script_output],
                sorted_inputs: blueprint_sorted_inputs,
                outputs,
                sorted_mints: vec![],
                withdrawal: None,
                fee_buffer: HARVESTING_TX_FEE_DELTA,
                operator_address: operator_address.clone(),
            };
            let BlueprintEstimates {
                estimated_fee,
                change_output,
                ..
            } = blueprint.compute_estimated_fee_and_change_output();

            // The change-output will be evenly distributed amongst all payout receivers.
            let chg_output_coin = change_output.output.value().coin;
            let amt = chg_output_coin / num_payouts;
            for output in blueprint.outputs.iter_mut().skip(1) {
                output.output.value_mut().coin += amt;
            }

            // Add residual amount to the last output
            blueprint.outputs.last_mut().unwrap().output.value_mut().coin += chg_output_coin % num_payouts;

            let mut tx_builder = blueprint.build(estimated_fee, None);
            tx_builder
                .add_collateral(InputBuilderResult::from(self.ctx.select::<Collateral>()))
                .unwrap();
            let inputs = tx_builder
                .get_inputs()
                .clone()
                .into_iter()
                .zip(sorted_input_data)
                .map(
                    |(TransactionUnspentOutput { input, output }, InputData { issued_at, .. })| {
                        CardanoTxInput {
                            output_ref: OutputRef::new(input.transaction_id, input.index),
                            tx_output: output,
                            issued_at,
                        }
                    },
                )
                .collect();

            let output = tx_builder
                .build(ChangeSelectionAlgo::Default, &operator_address)
                .unwrap();

            let tx_body = output.body();
            let tx_hash = hash_transaction_canonical(&tx_body);

            let resolved_tx = PartiallySignedCardanoTx {
                tx: output.build_unchecked(),
                inputs,
            };

            let mut buffer_wallet = buffer_wallet_in;
            buffer_wallet.balance -= batch.total_payout;
            let buffer_wallet_out_output_ref = OutputRef::new(tx_hash, 0);
            buffer_wallet.state_id = buffer_wallet_out_output_ref;
            let buffer_wallet_bearer = FinalizedTxOut(bw_out, buffer_wallet_out_output_ref);

            let predicted_buffer_wallet_update = EntityUpdated {
                consumed: Some(bw_in_output_ref),
                created: (buffer_wallet, buffer_wallet_bearer),
            };

            let executed_tasks = predicted_harvest_order_spends
                .iter()
                .map(|(id, _)| (*id).into())
                .collect();

            let output = HarvestFlowEntityUpdates {
                predicted_buffer_wallet_update,
                predicted_harvest_order_spends,
                predicted_merkle_tree: batch.input_merkle_tree, // TODO: DEX-935
                resolved_tx,
            };

            Ok(ExecutionResult {
                executed_tasks,
                output,
            })
        } else {
            panic!("No harvest batch exists (didn't call .feed())")
        }
    }
}

#[derive(Clone)]
pub struct BufferingFlow<GaugeId, StateId, Bearer, Ctx, OnChainIndex, FundingIndex> {
    onchain_index: OnChainIndex,
    funding_index: FundingIndex,
    batch: Option<BufferingBatch<GaugeId, StateId, Bearer>>,
    ctx: Ctx,
}

#[async_trait]
impl<Ctx, OnChainIndex, FundingIndex>
    BatchExecutor<
        TaskId,
        GaugeBuffering<FarmId>,
        BufferingFlowEntityUpdates<OutputRef, FarmId, FinalizedTxOut, Transaction, CardanoTxInputs>,
        Error,
    > for BufferingFlow<FarmId, OutputRef, FinalizedTxOut, Ctx, OnChainIndex, FundingIndex>
where
    Ctx: Send
        + Clone
        + Has<BufferWalletScript>
        + Has<Collateral>
        + Has<OperatorCreds>
        + Has<DeployedValidator<{ SmartFarm as u8 }>>
        + Has<DeployedValidator<{ PermManager as u8 }>>
        + Has<SplashPolicy>,
    OnChainIndex: GaugeIndex<FarmId, OutputRef, FinalizedTxOut>
        + AuthManagerIndex<FarmId, OutputRef, FinalizedTxOut>
        + BufferWalletIndex<OutputRef, FinalizedTxOut>
        + Send,
    FundingIndex: FundingRepo + Send + Clone,
{
    async fn feed(&mut self, task_id: TaskId, task: GaugeBuffering<FarmId>) -> Control<TaskId> {
        let batch = if let Some(ref mut batch) = self.batch {
            batch
        } else if let Some(bw) = self.onchain_index.get_buffer_wallet().await {
            let bundled = Bundled(bw.0.wallet, bw.1);
            if let Some(auth_manager) = self.onchain_index.get_auth_manager().await {
                self.batch.insert(BufferingBatch::new(bundled, auth_manager))
            } else {
                error!("No auth manager found");
                return Control::Stop;
            }
        } else {
            error!("No buffer wallet found");
            return Control::Stop;
        };
        if let Some(gauge) = self.onchain_index.get_gauge(task.gauge_id).await {
            batch.add_gauge(gauge);
        } else {
            warn!("Gauge {} not found, dropping task {}", task.gauge_id, task_id);
            return Control::Drop(task_id);
        }
        Control::Next
    }

    async fn execute(
        &mut self,
    ) -> Result<
        ExecutionResult<
            TaskId,
            BufferingFlowEntityUpdates<OutputRef, FarmId, FinalizedTxOut, Transaction, CardanoTxInputs>,
        >,
        Error,
    > {
        use spectrum_offchain::ledger::IntoLedger;
        enum RefInputT {
            AuthManager,
            Gauge,
        }
        if let Some(batch) = self.batch.take() {
            let buffer_wallet_script = self.ctx.select::<BufferWalletScript>().0;
            let Bundled(_, FinalizedTxOut(bw_tx_out, bw_in_output_ref)) = batch.buffer_wallet;
            let buffer_wallet_input =
                SingleInputBuilder::new(TransactionInput::from(bw_in_output_ref), bw_tx_out.clone())
                    .native_script(
                        buffer_wallet_script.clone(),
                        NativeScriptWitnessInfo::num_signatures(2),
                    )
                    .unwrap();

            let smart_farm_deployed_validator = self.ctx.select::<DeployedValidator<{ SmartFarm as u8 }>>();

            let smart_farm_ref_input = smart_farm_deployed_validator.reference_utxo;

            let Bundled(_, tx_out) = batch.auth_manager;

            let perm_manager_unspent_input =
                TransactionUnspentOutput::new(TransactionInput::from(tx_out.1), tx_out.0);

            let mut typed_ref_inputs = vec![
                (RefInputT::AuthManager, perm_manager_unspent_input),
                (RefInputT::Gauge, smart_farm_ref_input),
            ];
            typed_ref_inputs.sort_by_key(|(_, input)| input.input.clone());

            let perm_manager_input_ix = if matches!(typed_ref_inputs[0].0, RefInputT::AuthManager) {
                0
            } else {
                1
            };

            let reference_inputs: Vec<_> = typed_ref_inputs
                .into_iter()
                .map(|(_, tx_unspent_output)| tx_unspent_output)
                .collect();

            let mut spent_predicted = vec![];
            let mut spent_confirmed = vec![];
            let funding_boxes = self
                .funding_index
                .collect()
                .await
                .map(|available_boxes| {
                    assert!(
                        available_boxes.total_lovelaces() > GAUGE_BUFFERING_TX_MINIMAL_FUNDING_BOX_BALANCE
                    );
                    let AvailableFundingBoxes { confirmed, predicted } = available_boxes;
                    let mut boxes = vec![];

                    enum Mod {
                        Confirmed,
                        Predicted,
                    }

                    let confirmed = confirmed.into_iter().map(|f| (Mod::Confirmed, f));
                    let predicted = predicted.into_iter().map(|f| (Mod::Predicted, f));

                    let mut value = 0;
                    for (m, f) in confirmed.into_iter().chain(predicted) {
                        value += f.value.coin;
                        let output_ref = f.id.into();

                        if let Mod::Confirmed = m {
                            spent_confirmed.push(f.id);
                        } else {
                            spent_predicted.push(f.id);
                        }

                        let tx_output = f.into_ledger(self.ctx.clone());
                        let input = SingleInputBuilder::new(TransactionInput::from(output_ref), tx_output)
                            .payment_key()
                            .unwrap();
                        boxes.push((InputT::FundingBox(input), output_ref));

                        if value > GAUGE_BUFFERING_TX_MINIMAL_FUNDING_BOX_BALANCE {
                            break;
                        }
                    }

                    boxes
                })
                .unwrap()
                .into_iter();

            enum InputT<S> {
                Gauge(S),
                BufferWallet(InputBuilderResult),
                FundingBox(InputBuilderResult),
            }
            let mut typed_inputs: Vec<_> = batch
                .gauges
                .into_iter()
                .map(|g| {
                    let output_ref = g.0.state_id;
                    (InputT::Gauge(g), output_ref)
                })
                .chain(funding_boxes)
                .chain([(InputT::BufferWallet(buffer_wallet_input), bw_in_output_ref)])
                .collect();
            typed_inputs.sort_by_key(|(_, input)| *input);

            // Now extract gauge inputs in sorted order
            let sorted_gauge_inputs: Vec<_> = typed_inputs
                .iter()
                .filter_map(|(typ, _)| {
                    if let InputT::Gauge(Bundled(g, _)) = typ {
                        return Some(g.clone());
                    }
                    None
                })
                .collect();

            let mut total_splash_to_deposit = 0;
            let splash_asset_name = AssetName::from_utf8(SPLASH_NAME.into());
            let splash_policy = self.ctx.select::<SplashPolicy>().0;
            let splash_asset_class = AssetClass::Token(Token(splash_policy, splash_asset_name));

            let mut buffer_wallet_out = bw_tx_out;

            let gauge_script_hash = smart_farm_deployed_validator.hash;
            let gauge_ex_units = Some(DaoScriptData::global().mint_farm_auth_token.ex_units.clone());

            // The TX output is arranged as:
            //   [buffer_wallet_output] <> gauge_outputs <> [change_output],
            // where the gauge_outputs are ordered in like-manner to the sorted gauge-inputs: the
            // first gauge-input is associated with output index 1, the second with output index 2
            // etc.

            // This variable associates a given gauge-input with its associated output index. Needed
            // by the input's redeemer.
            let mut successor_out_ix = 1;
            let mut gauge_outputs = vec![];
            let sorted_inputs: Vec<_> = typed_inputs
                .into_iter()
                .map(|(input, _)| match input {
                    InputT::Gauge(Bundled(g, tx_out)) => {
                        total_splash_to_deposit += g.balance;
                        let gauge_redeemer = smart_farm::Redeemer {
                            successor_out_ix,
                            action: smart_farm::Action::DistributeRewards {
                                perm_manager_input_ix,
                            },
                        }
                        .into_pd();
                        let gauge_witness = PartialPlutusWitness::new(
                            PlutusScriptWitness::Ref(gauge_script_hash),
                            gauge_redeemer,
                        );

                        let gauge_input =
                            SingleInputBuilder::new(TransactionInput::from(tx_out.1), tx_out.0.clone())
                                .plutus_script_inline_datum(gauge_witness, RequiredSigners::from(vec![]))
                                .unwrap();

                        let amount_splash_in_gauge = tx_out.0.value().amount_of(splash_asset_class).unwrap();
                        let splash_delta = make_splash_value(splash_asset_class, amount_splash_in_gauge);
                        assert!(buffer_wallet_out.value_mut().checked_add(&splash_delta).is_ok());

                        let mut gauge_output = tx_out.0;
                        assert!(gauge_output.value_mut().checked_sub(&splash_delta).is_ok());

                        gauge_outputs.push(gauge_output);

                        successor_out_ix += 1;
                        (gauge_input, gauge_ex_units.clone())
                    }
                    InputT::BufferWallet(b) | InputT::FundingBox(b) => (b, None),
                })
                .collect();

            let buffer_wallet_out_balance = buffer_wallet_out.value().amount_of(splash_asset_class).unwrap();

            let outputs: Vec<_> = std::iter::once(buffer_wallet_out.clone())
                .chain(gauge_outputs.clone())
                .map(SingleOutputBuilderResult::new)
                .collect();
            let OperatorCreds(_, operator_address) = self.ctx.select::<OperatorCreds>();

            let change_output_ix = outputs.len() as u64;
            let blueprint = DaoTxBlueprint {
                reference_inputs,
                sorted_inputs,
                outputs,
                sorted_mints: vec![],
                withdrawal: None,
                fee_buffer: GAUGE_BUFFERING_TX_FEE_DELTA,
                operator_address: operator_address.clone(),
            };
            let BlueprintEstimates {
                estimated_fee,
                change_output,
                ..
            } = blueprint.compute_estimated_fee_and_change_output();

            assert!(!change_output.output.value().has_multiassets());
            let change_output_value = change_output.output.value().clone();
            // Bot will pocket the change-output, since it's paying the TX fee
            let mut tx_builder = blueprint.build(estimated_fee, Some(change_output));
            tx_builder
                .add_collateral(InputBuilderResult::from(self.ctx.select::<Collateral>()))
                .unwrap();
            let inputs = tx_builder
                .get_inputs()
                .clone()
                .into_iter()
                .map(|TransactionUnspentOutput { input, output }| CardanoTxInput {
                    output_ref: OutputRef::new(input.transaction_id, input.index),
                    tx_output: output,
                    issued_at: None,
                })
                .collect();
            let output = tx_builder
                .build(ChangeSelectionAlgo::Default, &operator_address)
                .unwrap();

            let tx_body = output.body();
            let tx_hash = hash_transaction_canonical(&tx_body);

            let resolved_tx = PartiallySignedCardanoTx {
                tx: output.build_unchecked(),
                inputs,
            };

            // Gather predicted buffer_wallet update
            let buffer_wallet_out_output_ref = OutputRef::new(tx_hash, 0);
            let buffer_wallet = BufferWallet {
                state_id: buffer_wallet_out_output_ref,
                balance: buffer_wallet_out_balance,
            };
            let buffer_wallet_bearer = FinalizedTxOut(buffer_wallet_out, buffer_wallet_out_output_ref);

            let predicted_buffer_wallet_update = EntityUpdated {
                consumed: Some(bw_in_output_ref),
                created: (buffer_wallet, buffer_wallet_bearer),
            };

            // Gather predicted gauge updates
            assert_eq!(sorted_gauge_inputs.len(), gauge_outputs.len());
            let predicted_gauge_updates: Vec<_> = sorted_gauge_inputs
                .into_iter()
                .zip(gauge_outputs.into_iter())
                .enumerate()
                .map(|(ix, (gauge_in, gauge_output))| {
                    let output_ref = OutputRef::new(tx_hash, (ix as u64) + 1);
                    let gauge_out_bearer = FinalizedTxOut(gauge_output, output_ref);
                    let gauge_out = Gauge {
                        id: gauge_in.id,
                        state_id: output_ref,
                        balance: 0,
                    };
                    EntityUpdated {
                        consumed: Some(gauge_in.state_id),
                        created: (gauge_out, gauge_out_bearer),
                    }
                })
                .collect();

            // Funding box changes
            let funding_id = FundingBoxId::from(OutputRef::new(tx_hash, change_output_ix));
            let created_funding_box = vec![Predicted(FundingBox {
                value: change_output_value,
                id: funding_id,
            })];

            let funding_box_changes = Some(FundingBoxChanges {
                spent_predicted,
                spent_confirmed,
                created: created_funding_box,
            });

            let executed_tasks: Vec<TaskId> = predicted_gauge_updates
                .iter()
                .map(|g| g.created.0.id.into())
                .collect();

            let output = BufferingFlowEntityUpdates {
                predicted_gauge_updates,
                predicted_buffer_wallet_update,
                funding_box_changes,
                resolved_tx,
            };

            Ok(ExecutionResult {
                executed_tasks,
                output,
            })
        } else {
            panic!("No gauge buffering batch exists (didn't call .feed())")
        }
    }
}

fn make_splash_value(splash_asset_class: AssetClass, amount: u64) -> Value {
    let mut splash_tokens_value = Value::zero();
    splash_tokens_value.add_unsafe(splash_asset_class, amount);
    splash_tokens_value
}

#[derive(Clone)]
pub enum Flow<GaugeId, StateId, Bearer, Ctx, OnChainIndex, FundingIndex, PositionIndex> {
    Harvesting(HarvestingFlow<StateId, Bearer, Ctx, PositionIndex, OnChainIndex>),
    Buffering(BufferingFlow<GaugeId, StateId, Bearer, Ctx, OnChainIndex, FundingIndex>),
}

#[derive(Clone)]
pub struct Executor<
    GaugeId,
    StateId,
    Bearer,
    Tx,
    TxInputs,
    Ctx,
    TxErr,
    PositionIndex,
    OnChainIndex,
    FundingIndex,
    TxSubmit,
    Verifier,
> {
    position_index: PositionIndex,
    onchain_index: OnChainIndex,
    funding_index: FundingIndex,
    tx_submit: TxSubmit,
    verifier: Verifier,
    ctx: Ctx,
    blocked_on: Option<Flow<GaugeId, StateId, Bearer, Ctx, OnChainIndex, FundingIndex, PositionIndex>>,
    pd: PhantomData<(Tx, TxInputs, TxErr)>,
}

impl<
        GaugeId,
        StateId,
        Bearer,
        Tx,
        TxInputs,
        Ctx,
        TxErr,
        PositionIndex,
        OnChainIndex,
        FundingIndex,
        TxSubmit,
        Verifier,
    >
    Executor<
        GaugeId,
        StateId,
        Bearer,
        Tx,
        TxInputs,
        Ctx,
        TxErr,
        PositionIndex,
        OnChainIndex,
        FundingIndex,
        TxSubmit,
        Verifier,
    >
{
    pub fn new(
        position_index: PositionIndex,
        onchain_index: OnChainIndex,
        funding_index: FundingIndex,
        tx_submit: TxSubmit,
        verifier: Verifier,
        ctx: Ctx,
    ) -> Self {
        Self {
            position_index,
            onchain_index,
            funding_index,
            tx_submit,
            verifier,
            ctx,
            blocked_on: None,
            pd: PhantomData,
        }
    }
}

#[async_trait]
impl<
        GaugeId,
        StateId,
        Bearer,
        Tx,
        TxInputs,
        Ctx,
        PositionIndex,
        OnChainIndex,
        FundingIndex,
        TxSubmit,
        Verifier,
    > BatchExecutor<TaskId, Task<GaugeId, StateId>, TransactionHash, Error>
    for Executor<
        GaugeId,
        StateId,
        Bearer,
        Tx,
        TxInputs,
        Ctx,
        Error,
        PositionIndex,
        OnChainIndex,
        FundingIndex,
        TxSubmit,
        Verifier,
    >
where
    GaugeId: Into<TaskId> + Copy + Send + Sync + Display + 'static,
    StateId:
        Into<OutputRef> + Copy + Eq + Hash + Send + Sync + Display + Serialize + DeserializeOwned + 'static,
    Bearer: Clone + Send + Sync + Serialize + DeserializeOwned + 'static,
    Tx: Send + Clone + CanonicalHash<Hash = TransactionHash>,
    TxInputs: Send + Clone,
    PositionIndex: Accounts<StateId> + Clone + Send,
    OnChainIndex: GaugeIndex<GaugeId, StateId, Bearer>
        + HarvestOrderIndex<StateId, Bearer>
        + BufferWalletIndex<StateId, Bearer>
        + Clone
        + Send
        + Sync,
    FundingIndex: FundingRepo + Clone + Send + Sync,
    TxSubmit: Clone + Network<Tx, RejectReasons> + Send,
    Verifier: RemoteVerifier<PartiallySignedTx<Tx, TxInputs>, Tx> + Send,
    HarvestingFlow<StateId, Bearer, Ctx, PositionIndex, OnChainIndex>: BatchExecutor<
        TaskId,
        Harvesting<StateId>,
        HarvestFlowEntityUpdates<StateId, Bearer, Tx, TxInputs>,
        Error,
    >,
    BufferingFlow<GaugeId, StateId, Bearer, Ctx, OnChainIndex, FundingIndex>: BatchExecutor<
        TaskId,
        GaugeBuffering<GaugeId>,
        BufferingFlowEntityUpdates<StateId, GaugeId, Bearer, Tx, TxInputs>,
        Error,
    >,
    Ctx: Send
        + Clone
        + Has<BufferWalletScript>
        + Has<DeployedValidator<{ HarvestOrder as u8 }>>
        + Has<NetworkId>,
{
    async fn feed(&mut self, task_id: TaskId, task: Task<GaugeId, StateId>) -> Control<TaskId> {
        let flow = match self.blocked_on {
            None => match task {
                Task::GaugeBuffering(_) => self.blocked_on.insert(Flow::Buffering(BufferingFlow {
                    onchain_index: self.onchain_index.clone(),
                    funding_index: self.funding_index.clone(),
                    batch: None,
                    ctx: self.ctx.clone(),
                })),
                Task::Harvesting(_) => self.blocked_on.insert(Flow::Harvesting(HarvestingFlow {
                    position_index: self.position_index.clone(),
                    onchain_index: self.onchain_index.clone(),
                    batch: None,
                    ctx: self.ctx.clone(),
                })),
            },
            Some(ref mut flow) => flow,
        };
        match (flow, task) {
            (Flow::Harvesting(hf), Task::Harvesting(ht)) => hf.feed(task_id, ht).await,
            (Flow::Buffering(bf), Task::GaugeBuffering(bt)) => bf.feed(task_id, bt).await,
            _ => Control::Next,
        }
    }

    async fn execute(&mut self) -> Result<ExecutionResult<TaskId, TransactionHash>, Error> {
        match self.blocked_on.take() {
            Some(flow) => {
                let (typed_result, resolved_tx, executed_tasks) = match flow {
                    Flow::Harvesting(mut hf) => {
                        let res = hf.execute().await?;
                        let resolved_tx = res.output.resolved_tx.clone();
                        let typed_res = TypedExecutionUpdate::Harvesting(res.output);
                        (typed_res, resolved_tx, res.executed_tasks)
                    }
                    Flow::Buffering(mut bf) => {
                        let res = bf.execute().await?;
                        let resolved_tx = res.output.resolved_tx.clone();
                        let typed_res = TypedExecutionUpdate::Buffering(res.output);
                        (typed_res, resolved_tx, res.executed_tasks)
                    }
                };

                loop {
                    match self.verifier.try_approve(&resolved_tx).await {
                        Ok(coop_tx) => match (self.tx_submit.submit_tx(coop_tx).await, typed_result) {
                            (Ok(_), update) => {
                                index_predicted_entities(update, &self.onchain_index, &self.funding_index)
                                    .await;
                                return Ok(ExecutionResult {
                                    executed_tasks,
                                    output: resolved_tx.tx.canonical_hash(),
                                });
                            }

                            (Err(reject_reasons), update) => {
                                if let Some(failed_task_ids) = extract_failed_task_ids(update, reject_reasons)
                                {
                                    return Err(Error::TxInputsAlreadySpent { failed_task_ids });
                                } else {
                                    return Err(Error::UnrecoverableNodeError);
                                }
                            }
                        },
                        Err(VerifierRejection::Unavailable) => continue, // todo: RestartVerifierWhenUnresponsive
                        Err(VerifierRejection::InvalidWithdrawal) => {
                            panic!() // todo: ShouldGoToIndexFaultMode; ShouldResyncOnMismatch
                        }
                    }
                }
            }
            None => panic!("No flow instance exists"),
        }
    }
}

enum TypedExecutionUpdate<HarvestOut, BufferingOut> {
    Harvesting(HarvestOut),
    Buffering(BufferingOut),
}

async fn index_predicted_entities<StateId, GaugeId, Bearer, OnChainIndex, FundingIndex, Tx, TxInputs>(
    update: TypedExecutionUpdate<
        HarvestFlowEntityUpdates<StateId, Bearer, Tx, TxInputs>,
        BufferingFlowEntityUpdates<StateId, GaugeId, Bearer, Tx, TxInputs>,
    >,
    onchain_index: &OnChainIndex,
    funding_index: &FundingIndex,
) where
    GaugeId: Into<TaskId> + Copy + Send + Sync + Display + 'static,
    StateId:
        Into<OutputRef> + Copy + Eq + Hash + Send + Sync + Display + Serialize + DeserializeOwned + 'static,
    Bearer: Clone + Send + Sync + Serialize + DeserializeOwned + 'static,
    OnChainIndex: GaugeIndex<GaugeId, StateId, Bearer>
        + HarvestOrderIndex<StateId, Bearer>
        + BufferWalletIndex<StateId, Bearer>
        + Clone
        + Send,
    FundingIndex: FundingRepo + Clone + Send,
{
    match update {
        TypedExecutionUpdate::Harvesting(HarvestFlowEntityUpdates {
            predicted_buffer_wallet_update,
            predicted_harvest_order_spends,
            predicted_merkle_tree,
            ..
        }) => {
            for (output_ref, spend) in &predicted_harvest_order_spends {
                onchain_index
                    .write_predicted_spend_harvest_order(*output_ref, spend)
                    .await;
            }

            let EntityUpdated {
                consumed,
                created: (wallet, bearer),
            } = predicted_buffer_wallet_update.clone();
            let wallet_wrap = BufferWalletWrap {
                wallet,
                predicted_merkle_tree: Some(predicted_merkle_tree),
            };
            onchain_index
                .write_predicted_buffer_wallet(Bundled(wallet_wrap, bearer), consumed)
                .await;
        }
        TypedExecutionUpdate::Buffering(BufferingFlowEntityUpdates {
            predicted_gauge_updates,
            predicted_buffer_wallet_update,
            funding_box_changes,
            ..
        }) => {
            for gauge_update in &predicted_gauge_updates {
                let EntityUpdated {
                    consumed,
                    created: (gauge, bearer),
                } = gauge_update.clone();
                onchain_index
                    .write_predicted_gauge(Bundled(gauge, bearer), consumed)
                    .await;
            }

            let EntityUpdated {
                consumed,
                created: (wallet, bearer),
            } = predicted_buffer_wallet_update.clone();
            let wallet_wrap = BufferWalletWrap {
                wallet,
                predicted_merkle_tree: None,
            };
            onchain_index
                .write_predicted_buffer_wallet(Bundled(wallet_wrap, bearer), consumed)
                .await;

            if let Some(FundingBoxChanges {
                spent_predicted,
                spent_confirmed,
                created,
            }) = &funding_box_changes
            {
                for id in spent_predicted {
                    funding_index.spend_predicted(*id).await;
                }

                for id in spent_confirmed {
                    funding_index.spend_confirmed(*id).await;
                }

                for funding_box in created {
                    funding_index.put_predicted(funding_box.clone()).await;
                }
            }
        }
    }
}

/// Returns `TaskId`s of inputs that have been already spent on-chain.
fn extract_failed_task_ids<StateId, GaugeId, Bearer, Tx, TxInputs>(
    update: TypedExecutionUpdate<
        HarvestFlowEntityUpdates<StateId, Bearer, Tx, TxInputs>,
        BufferingFlowEntityUpdates<StateId, GaugeId, Bearer, Tx, TxInputs>,
    >,
    reject_reasons: RejectReasons,
) -> Option<Vec<TaskId>>
where
    GaugeId: Into<TaskId> + Copy + Send + Sync + Display + 'static,
    StateId:
        Into<OutputRef> + Copy + Eq + Hash + Send + Sync + Display + Serialize + DeserializeOwned + 'static,
    Bearer: Clone + Send + Sync + Serialize + DeserializeOwned + 'static,
{
    let already_spent_inputs = extract_already_spent_inputs(reject_reasons)?;
    match update {
        TypedExecutionUpdate::Harvesting(HarvestFlowEntityUpdates {
            predicted_harvest_order_spends,
            ..
        }) => {
            let failed_task_ids: Vec<TaskId> = predicted_harvest_order_spends
                .iter()
                .filter_map(|(id, _)| {
                    let output_ref = (*id).into();
                    if already_spent_inputs.contains(&output_ref) {
                        return Some(output_ref.into());
                    }
                    None
                })
                .collect::<Vec<_>>();

            // TODO (DEX-920): if buffer wallet was already spent, the reward bot ideally needs to
            // wait until next block to get the latest version of the wallet. Not doing so will just
            // lead to the next formed TX to be rejected right here again.
            Some(failed_task_ids)
        }
        TypedExecutionUpdate::Buffering(BufferingFlowEntityUpdates {
            predicted_gauge_updates,
            ..
        }) => {
            let failed_task_ids: Vec<TaskId> = predicted_gauge_updates
                .iter()
                .filter_map(|e| {
                    if let Some(consumed_input) = e.consumed {
                        if already_spent_inputs.contains(&consumed_input.into()) {
                            return Some(e.created.0.id.into());
                        }
                    }
                    None
                })
                .collect();
            Some(failed_task_ids)
        }
    }
}

fn extract_already_spent_inputs(RejectReasons(reasons): RejectReasons) -> Option<Vec<OutputRef>> {
    reasons.map(|ApplyTxError { node_errors }| {
        node_errors
            .iter()
            .filter_map(|err| {
                if let ConwayLedgerPredFailure::UtxowFailure(ConwayUtxowPredFailure::UtxoFailure(
                    ConwayUtxoPredFailure::BadInputsUtxo(bad_inputs),
                )) = err
                {
                    Some(
                        bad_inputs
                            .iter()
                            .map(|TxInput { tx_hash, index }| {
                                OutputRef::new(
                                    TransactionHash::from_raw_bytes(tx_hash.as_slice()).unwrap(),
                                    *index,
                                )
                            })
                            .collect::<Vec<_>>(),
                    )
                } else {
                    None
                }
            })
            .flatten()
            .collect()
    })
}
