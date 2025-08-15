use crate::constants::{
    GAUGE_BUFFERING_TX_FEE_DELTA, GAUGE_BUFFERING_TX_MINIMAL_FUNDING_BOX_BALANCE,
    HARVESTING_TX_ASSUMED_BASE_FEE, HARVESTING_TX_FEE_DELTA,
};
use crate::emission::{reward_amount, Emission};
use crate::engine::batch::{BufferingBatch, HarvestBatch, OrderWithPayout};
use crate::engine::resolved_tx::{PartiallySignedCardanoTx, PartiallySignedTx};
use crate::engine::task::{GaugeBuffering, Harvesting, Task, TaskId};
use crate::engine::verifier::{RemoteVerifier, VerifierRejection};
use crate::entity_index::{AuthManagerIndex, HarvestOrderIndex};
use crate::entity_index::{BufferWalletIndex, GaugeIndex};
use crate::events::EntityUpdated;
use crate::onchain::buffer_wallet::BufferWallet;
use crate::onchain::harvest_order::{HarvestOrder, HarvestOrderAction};
use crate::onchain::smart_farm::Gauge;
use crate::positions::{AccountState, LockedByAnotherReq, Positions};
use async_trait::async_trait;
use bloom_offchain::execution_engine::bundled::Bundled;
use cml_chain::address::{BaseAddress, EnterpriseAddress};
use cml_chain::builders::input_builder::{InputBuilderResult, SingleInputBuilder};
use cml_chain::builders::output_builder::{SingleOutputBuilderResult, TransactionOutputBuilder};
use cml_chain::builders::tx_builder::{ChangeSelectionAlgo, SignedTxBuilder, TransactionUnspentOutput};
use cml_chain::builders::witness_builder::{
    NativeScriptWitnessInfo, PartialPlutusWitness, PlutusScriptWitness,
};
use cml_chain::certs::Credential;
use cml_chain::transaction::{Transaction, TransactionInput};
use cml_chain::{RequiredSigners, Value};
use cml_crypto::RawBytesEncoding;
use log::{error, warn};
use serde::de::DeserializeOwned;
use serde::Serialize;
use spectrum_cardano_lib::collateral::Collateral;
use spectrum_cardano_lib::hash::hash_transaction_canonical;
use spectrum_cardano_lib::output::FinalizedTxOut;
use spectrum_cardano_lib::plutus_data::IntoPlutusData;
use spectrum_cardano_lib::protocol_params::constant_tx_builder;
use spectrum_cardano_lib::transaction::TransactionOutputExtension;
use spectrum_cardano_lib::value::ValueExtension;
use spectrum_cardano_lib::{ex_units, AssetClass, AssetName, NetworkId, OutputRef, Token};
use spectrum_offchain::domain::event::Predicted;
use spectrum_offchain::domain::Has;
use spectrum_offchain::network::Network;
use splash_dao_offchain::constants::SPLASH_NAME;
use splash_dao_offchain::deployment::DaoScriptData;
use splash_dao_offchain::entities::onchain::funding_box::{FundingBox, FundingBoxId};
use splash_dao_offchain::entities::onchain::smart_farm::{self, FarmId};
use splash_dao_offchain::funding::{AvailableFundingBoxes, FundingRepo};
use splash_dao_offchain::protocol_config::{
    BufferWalletScript, FarmAuthPolicy, FarmAuthRefScriptOutput, HarvestOrderRefScriptOutput,
    HarvestOrderScriptHash, OperatorCreds, PermManagerBoxRefScriptOutput, SplashPolicy,
};
use splash_dao_offchain::routines::actions::{BlueprintEstimates, DaoTxBlueprint};
use splash_dao_offchain::routines::FundingBoxChanges;
use std::fmt::Display;
use std::hash::Hash;
use std::marker::PhantomData;

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
pub struct ExecutionResult<GaugeId, StateId, Bearer, TaskId, Out> {
    pub executed_tasks: Vec<TaskId>,
    pub output: Out,
    pub predicted_harvest_order_spends: Vec<StateId>,
    pub predicted_buffer_wallet_update: EntityUpdated<BufferWallet<StateId>, StateId, Bearer>,
    pub predicted_gauge_updates: Vec<EntityUpdated<Gauge<GaugeId, StateId>, StateId, Bearer>>,
    pub funding_box_changes: Option<FundingBoxChanges>,
}

#[async_trait]
pub trait BatchExecutor<GaugeId, StateId, Bearer, TaskId, Task, Out, Err> {
    async fn feed(&mut self, task_id: TaskId, task: Task) -> Control<TaskId>;
    async fn execute(&mut self) -> Result<ExecutionResult<GaugeId, StateId, Bearer, TaskId, Out>, Err>;
}

#[derive(Clone)]
pub struct HarvestingFlow<StateId, Bearer, Ctx, PositionIndex, OnChainIndex, Emission> {
    position_index: PositionIndex,
    onchain_index: OnChainIndex,
    emission: Emission,
    batch: Option<HarvestBatch<StateId, Bearer>>,
    ctx: Ctx,
}

#[async_trait]
impl<Ctx, PositionIndex, OnChainIndex, Emiss>
    BatchExecutor<TaskId, Harvesting<OutputRef>, PartiallySignedCardanoTx, ()>
    for HarvestingFlow<OutputRef, FinalizedTxOut, Ctx, PositionIndex, OnChainIndex, Emiss>
where
    PositionIndex: Positions<OutputRef> + Send,
    OnChainIndex:
        BufferWalletIndex<OutputRef, FinalizedTxOut> + HarvestOrderIndex<OutputRef, FinalizedTxOut> + Send,
    Emiss: Emission + Send,
    Ctx: Send
        + Has<BufferWalletScript>
        + Has<HarvestOrderScriptHash>
        + Has<HarvestOrderRefScriptOutput>
        + Has<SplashPolicy>
        + Has<OperatorCreds>
        + Has<Collateral>
        + Has<NetworkId>,
{
    async fn feed(&mut self, task_id: TaskId, task: Harvesting<OutputRef>) -> Control<TaskId> {
        let order = if let Some(order) = self.onchain_index.read_harvest_order(task.order_id).await {
            order
        } else {
            return Control::Drop(task_id);
        };
        let batch = if let Some(ref mut batch) = self.batch {
            batch
        } else {
            if let Some(bw) = self.onchain_index.get_buffer_wallet().await {
                self.batch.insert(HarvestBatch::new(bw))
            } else {
                error!("No buffer wallet found");
                return Control::Stop;
            }
        };
        let (req, tx_out) = match order {
            crate::entity_index::Mod::Confirmed(Bundled(req, tx_out))
            | crate::entity_index::Mod::Predicted(Bundled(req, tx_out)) => (req.0, tx_out),
        };
        match self
            .position_index
            .query_account(&Credential::new_pub_key(req.account_key))
            .await
        {
            Ok(AccountState {
                activated_at,
                total_share_bps,
            }) => {
                let emission = self
                    .emission
                    .total_emission_between(activated_at, req.issued_at.0);
                let payout = reward_amount(total_share_bps, emission);
                if batch.can_accept(payout) {
                    if let Err(LockedByAnotherReq(concurrent_req)) = self
                        .position_index
                        .lock_account(&req.id, &Credential::new_pub_key(req.account_key))
                        .await
                    {
                        warn!(
                            "Account {} is already locked by another request {}, dropping request {}",
                            hex::encode(req.account_key.to_raw_bytes()),
                            concurrent_req,
                            req.id
                        );
                        return Control::Drop(task_id);
                    }
                    batch.add_order(Bundled(req, tx_out), payout);
                } else {
                    warn!(
                        "Buffer wallet is running out of funds, cannot process request {}",
                        req.id
                    );
                    return Control::Stop;
                }
            }
            Err(_not_found) => {
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
    ) -> Result<ExecutionResult<FarmId, OutputRef, FinalizedTxOut, TaskId, PartiallySignedCardanoTx>, ()>
    {
        if let Some(batch) = self.batch.take() {
            let harvest_order_ref_script_output = self.ctx.select::<HarvestOrderRefScriptOutput>().0;

            let num_payouts = batch.orders.len() as u64;

            let buffer_wallet_script = self.ctx.select::<BufferWalletScript>().0;

            let mut accounts = vec![];

            let harvest_order_redeemer = HarvestOrderAction::Harvest.into_pd();
            let harvest_order_script_hash = self.ctx.select::<HarvestOrderScriptHash>().0;
            let harvest_order_witness = PartialPlutusWitness::new(
                PlutusScriptWitness::Ref(harvest_order_script_hash),
                harvest_order_redeemer,
            );
            let ex_units = Some(DaoScriptData::global().harvest_order.ex_units.clone());
            let network_id = self.ctx.select::<NetworkId>();
            let mut predicted_harvest_order_spends = vec![];
            let mut sorted_inputs: Vec<_> = batch
                .orders
                .into_iter()
                .map(
                    |OrderWithPayout {
                         order: Bundled(HarvestOrder { reward_receiver, .. }, tx_out),
                         payout,
                     }| {
                        let reward_receiver = reward_receiver.to_address(network_id);
                        accounts.push((reward_receiver, payout, tx_out.0.value().coin));
                        let harvest_order_input =
                            SingleInputBuilder::new(TransactionInput::from(tx_out.1), tx_out.0)
                                .plutus_script_inline_datum(
                                    harvest_order_witness.clone(),
                                    RequiredSigners::from(vec![]),
                                )
                                .unwrap();

                        predicted_harvest_order_spends.push(tx_out.1);

                        (harvest_order_input, ex_units.clone())
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
            sorted_inputs.push((buffer_wallet_input, None));

            sorted_inputs.sort_by_key(|(input, _)| input.input.clone());

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
                sorted_inputs,
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
                .map(|TransactionUnspentOutput { input, output }| {
                    (OutputRef::new(input.transaction_id, input.index), output)
                })
                .collect();

            let output = tx_builder
                .build(ChangeSelectionAlgo::Default, &operator_address)
                .unwrap();

            let tx_body = output.body();
            let tx_hash = hash_transaction_canonical(&tx_body);
            let tx_hash_bytes = <[u8; 32]>::from(tx_hash);
            let task_id = TaskId::from(tx_hash_bytes);

            let resolved_tx = PartiallySignedCardanoTx {
                tx: output.build_unchecked(),
                inputs,
            };

            let mut buffer_wallet = buffer_wallet_in.clone();
            buffer_wallet.balance -= batch.total_payout;
            let buffer_wallet_out_output_ref = OutputRef::new(tx_hash, 0);
            buffer_wallet.state_id = buffer_wallet_out_output_ref;
            let buffer_wallet_bearer = FinalizedTxOut(bw_out, buffer_wallet_out_output_ref);

            let predicted_buffer_wallet_update = EntityUpdated {
                consumed: Some(bw_in_output_ref),
                created: (buffer_wallet, buffer_wallet_bearer),
            };

            Ok(ExecutionResult {
                executed_tasks: vec![task_id], //todo: tasks map to orders
                output: resolved_tx,
                predicted_harvest_order_spends,
                predicted_buffer_wallet_update,
                predicted_gauge_updates: vec![],
                funding_box_changes: None,
            })
        } else {
            Err(())
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
impl<GaugeId, Ctx, OnChainIndex, FundingIndex>
    BatchExecutor<TaskId, GaugeBuffering<GaugeId>, PartiallySignedCardanoTx, ()>
    for BufferingFlow<GaugeId, OutputRef, FinalizedTxOut, Ctx, OnChainIndex, FundingIndex>
where
    Ctx: Send
        + Clone
        + Has<BufferWalletScript>
        + Has<Collateral>
        + Has<PermManagerBoxRefScriptOutput>
        + Has<FarmAuthRefScriptOutput>
        + Has<FarmAuthPolicy>
        + Has<OperatorCreds>
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
            if let Some(auth_manager) = self.onchain_index.get_auth_manager().await {
                self.batch.insert(BufferingBatch::new(bw, auth_manager))
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
    ) -> Result<ExecutionResult<FarmId, OutputRef, FinalizedTxOut, TaskId, PartiallySignedCardanoTx>, ()>
    {
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

            let smart_farm_ref_script = self.ctx.select::<FarmAuthRefScriptOutput>().0;

            let Bundled(_, tx_out) = batch.auth_manager;

            let perm_manager_unspent_input =
                TransactionUnspentOutput::new(TransactionInput::from(tx_out.1), tx_out.0);

            let mut typed_ref_inputs = vec![
                (RefInputT::AuthManager, perm_manager_unspent_input),
                (RefInputT::Gauge, smart_farm_ref_script),
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
                .filter_map(|(typ, e)| {
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

            let gauge_script_hash = self.ctx.select::<FarmAuthPolicy>().0;
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
                .map(|TransactionUnspentOutput { input, output }| {
                    (OutputRef::new(input.transaction_id, input.index), output)
                })
                .collect();
            let output = tx_builder
                .build(ChangeSelectionAlgo::Default, &operator_address)
                .unwrap();

            let tx_body = output.body();
            let tx_hash = hash_transaction_canonical(&tx_body);
            let tx_hash_bytes = <[u8; 32]>::from(tx_hash);
            let task_id = TaskId::from(tx_hash_bytes);

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
            let predicted_gauge_updates = sorted_gauge_inputs
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

            Ok(ExecutionResult {
                executed_tasks: vec![task_id], // todo: return correct task ids
                output: resolved_tx,
                predicted_harvest_order_spends: vec![],
                predicted_buffer_wallet_update,
                predicted_gauge_updates,
                funding_box_changes,
            })
        } else {
            Err(())
        }
    }
}

fn make_splash_value(splash_asset_class: AssetClass, amount: u64) -> Value {
    let mut splash_tokens_value = Value::zero();
    splash_tokens_value.add_unsafe(splash_asset_class, amount);
    splash_tokens_value
}

#[derive(Clone)]
pub enum Flow<GaugeId, StateId, Bearer, Ctx, OnChainIndex, FundingIndex, PositionIndex, Emission> {
    Harvesting(HarvestingFlow<StateId, Bearer, Ctx, PositionIndex, OnChainIndex, Emission>),
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
    Emission,
    Verifier,
> {
    position_index: PositionIndex,
    onchain_index: OnChainIndex,
    funding_index: FundingIndex,
    tx_submit: TxSubmit,
    emission: Emission,
    verifier: Verifier,
    ctx: Ctx,
    blocked_on:
        Option<Flow<GaugeId, StateId, Bearer, Ctx, OnChainIndex, FundingIndex, PositionIndex, Emission>>,
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
        Emission,
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
        Emission,
        Verifier,
    >
{
    pub fn new(
        position_index: PositionIndex,
        onchain_index: OnChainIndex,
        funding_index: FundingIndex,
        tx_submit: TxSubmit,
        emission: Emission,
        verifier: Verifier,
        ctx: Ctx,
    ) -> Self {
        Self {
            position_index,
            onchain_index,
            funding_index,
            tx_submit,
            emission,
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
        TxErr,
        PositionIndex,
        OnChainIndex,
        FundingIndex,
        TxSubmit,
        Emiss,
        Verifier,
    > BatchExecutor<GaugeId, StateId, Bearer, TaskId, Task<GaugeId, StateId>, (), ()>
    for Executor<
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
        Emiss,
        Verifier,
    >
where
    GaugeId: Copy + Send + Sync + Display + 'static,
    StateId: Copy + Eq + Hash + Send + Sync + Display + Serialize + DeserializeOwned + 'static,
    Bearer: Clone + Send + Sync + Serialize + DeserializeOwned + 'static,
    Tx: Send,
    TxInputs: Send,
    TxErr: Send,
    PositionIndex: Positions<StateId> + Clone + Send,
    OnChainIndex: GaugeIndex<GaugeId, StateId, Bearer>
        + HarvestOrderIndex<StateId, Bearer>
        + BufferWalletIndex<StateId, Bearer>
        + Clone
        + Send,
    FundingIndex: FundingRepo + Clone + Send,
    TxSubmit: Clone + Network<Tx, TxErr> + Send,
    Emiss: Emission + Clone + Send,
    Verifier: RemoteVerifier<PartiallySignedTx<Tx, TxInputs>, Tx> + Send,
    HarvestingFlow<StateId, Bearer, Ctx, PositionIndex, OnChainIndex, Emiss>:
        BatchExecutor<TaskId, Harvesting<StateId>, PartiallySignedTx<Tx, TxInputs>, ()>,
    BufferingFlow<GaugeId, StateId, Bearer, Ctx, OnChainIndex, FundingIndex>:
        BatchExecutor<TaskId, GaugeBuffering<GaugeId>, PartiallySignedTx<Tx, TxInputs>, ()>,
    Ctx: Send
        + Clone
        + Has<BufferWalletScript>
        + Has<HarvestOrderScriptHash>
        + Has<HarvestOrderRefScriptOutput>
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
                    emission: self.emission.clone(),
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

    async fn execute(&mut self) -> Result<ExecutionResult<TaskId, ()>, ()> {
        match self.blocked_on.take() {
            None => Err(()),
            Some(flow) => {
                let res = match flow {
                    Flow::Harvesting(mut hf) => hf.execute().await?,
                    Flow::Buffering(mut bf) => bf.execute().await?,
                };
                loop {
                    match self.verifier.try_approve(&res.output).await {
                        Ok(coop_tx) => {
                            // Only index predicted-writes if TX is successfully submitted
                            match self.tx_submit.submit_tx(coop_tx).await {
                                Ok(_) => {
                                    let ExecutionResult {
                                        executed_tasks,
                                        predicted_harvest_order_spends,
                                        predicted_buffer_wallet_update,
                                        predicted_gauge_updates,
                                        funding_box_changes,
                                        ..
                                    } = res;
                                    for output_ref in &predicted_harvest_order_spends {
                                        self.onchain_index
                                            .write_predicted_spend_harvest_order(*output_ref)
                                            .await;
                                    }

                                    let EntityUpdated {
                                        consumed,
                                        created: (buffer_wallet, bearer),
                                    } = predicted_buffer_wallet_update.clone();
                                    self.onchain_index
                                        .write_predicted_buffer_wallet(
                                            Bundled(buffer_wallet, bearer),
                                            consumed,
                                        )
                                        .await;

                                    for gauge_update in &predicted_gauge_updates {
                                        let EntityUpdated {
                                            consumed,
                                            created: (gauge, bearer),
                                        } = gauge_update.clone();
                                        self.onchain_index
                                            .write_predicted_gauge(Bundled(gauge, bearer), consumed)
                                            .await;
                                    }

                                    if let Some(FundingBoxChanges {
                                        spent_predicted,
                                        spent_confirmed,
                                        created,
                                    }) = &funding_box_changes
                                    {
                                        for id in spent_predicted {
                                            self.funding_index.spend_predicted(*id).await;
                                        }

                                        for id in spent_confirmed {
                                            self.funding_index.spend_confirmed(*id).await;
                                        }

                                        for funding_box in created {
                                            self.funding_index.put_predicted(funding_box.clone()).await;
                                        }
                                    }

                                    return Ok(ExecutionResult {
                                        executed_tasks,
                                        output: (),
                                        predicted_harvest_order_spends,
                                        predicted_buffer_wallet_update,
                                        predicted_gauge_updates,
                                        funding_box_changes,
                                    });
                                }
                                Err(_) => {
                                    return Err(());
                                }
                            }
                        }
                        Err(VerifierRejection::Unavailable) => continue, // todo: RestartVerifierWhenUnresponsive
                        Err(VerifierRejection::InvalidWithdrawal) => {
                            panic!() // todo: ShouldGoToIndexFaultMode; ShouldResyncOnMismatch
                        }
                    }
                }
            }
        }
    }
}
