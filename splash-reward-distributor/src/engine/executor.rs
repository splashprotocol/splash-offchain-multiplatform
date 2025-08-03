use crate::emission::Emission;
use crate::engine::batch::{BufferingBatch, HarvestBatch, OrderWithPayout};
use crate::engine::task::{GaugeBuffering, Harvesting, Task, TaskId};
use crate::index::{BufferWalletIndex, GaugeIndex, OrderIndex};
use crate::onchain::harvest_order::{HarvestOrder, HarvestOrderAction};
use crate::positions::{AccountState, LockedByAnotherReq, Positions};
use async_trait::async_trait;
use bloom_offchain::execution_engine::bundled::Bundled;
use cml_chain::address::{BaseAddress, EnterpriseAddress};
use cml_chain::builders::input_builder::{InputBuilderResult, SingleInputBuilder};
use cml_chain::builders::output_builder::{SingleOutputBuilderResult, TransactionOutputBuilder};
use cml_chain::builders::tx_builder::{ChangeSelectionAlgo, SignedTxBuilder};
use cml_chain::builders::witness_builder::{
    NativeScriptWitnessInfo, PartialPlutusWitness, PlutusScriptWitness,
};
use cml_chain::certs::{Credential, StakeCredential};
use cml_chain::plutus::ExUnits;
use cml_chain::transaction::TransactionInput;
use cml_chain::{RequiredSigners, Value};
use cml_crypto::RawBytesEncoding;
use log::{error, warn};
use spectrum_cardano_lib::collateral::Collateral;
use spectrum_cardano_lib::hash::hash_transaction_canonical;
use spectrum_cardano_lib::output::FinalizedTxOut;
use spectrum_cardano_lib::plutus_data::IntoPlutusData;
use spectrum_cardano_lib::protocol_params::constant_tx_builder;
use spectrum_cardano_lib::transaction::TransactionOutputExtension;
use spectrum_cardano_lib::value::ValueExtension;
use spectrum_cardano_lib::{AssetClass, AssetName, NetworkId, OutputRef, Token};
use spectrum_offchain::domain::Has;
use spectrum_offchain::network::Network;
use splash_dao_offchain::constants::SPLASH_NAME;
use splash_dao_offchain::deployment::DaoScriptData;
use splash_dao_offchain::protocol_config::{
    BufferWalletScript, HarvestOrderRefScriptOutput, HarvestOrderScriptHash, OperatorCreds, SplashPolicy,
};
use splash_dao_offchain::routines::actions::{BlueprintEstimates, DaoTxBlueprint};
use splash_reward_distributor::constants::{HARVESTING_TX_ASSUMED_BASE_FEE, HARVESTING_TX_FEE_DELTA};
use std::fmt::Display;
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
pub struct ExecutionResult<TaskId, Out> {
    pub executed_tasks: Vec<TaskId>,
    pub output: Out,
}

#[async_trait]
pub trait BatchExecutor<TaskId, Task, Out, Err> {
    async fn feed(&mut self, task_id: TaskId, task: Task) -> Control<TaskId>;
    async fn execute(&mut self) -> Result<ExecutionResult<TaskId, Out>, Err>;
}

pub struct HarvestingFlow<StateId, Bearer, Tx, Ctx, PositionIndex, OnChainIndex, Emission> {
    position_index: PositionIndex,
    onchain_index: OnChainIndex,
    emission: Emission,
    batch: Option<HarvestBatch<StateId, Bearer>>,
    ctx: Ctx,
    pd: PhantomData<Tx>,
}

#[async_trait]
impl<Ctx, PositionIndex, OnChainIndex, Emiss>
    BatchExecutor<TaskId, Harvesting<OutputRef>, SignedTxBuilder, ()>
    for HarvestingFlow<OutputRef, FinalizedTxOut, SignedTxBuilder, Ctx, PositionIndex, OnChainIndex, Emiss>
where
    PositionIndex: Positions<OutputRef> + Send,
    OnChainIndex: BufferWalletIndex<OutputRef, FinalizedTxOut> + OrderIndex<OutputRef, FinalizedTxOut> + Send,
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
        let order = if let Some(order) = self.onchain_index.get_order(task.order_id).await {
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
        let Bundled(req, _) = &order;
        match self
            .position_index
            .query_account(&Credential::new_pub_key(req.account))
            .await
        {
            Ok(AccountState {
                activated_at,
                queried_at,
                total_share_bps,
            }) => {
                let emission = self.emission.total_emission_between(activated_at, queried_at);
                let payout = reward_amount(total_share_bps, emission);
                if batch.can_accept(payout) {
                    if let Err(LockedByAnotherReq(concurrent_req)) = self
                        .position_index
                        .lock_account(&req.id, &Credential::new_pub_key(req.account))
                        .await
                    {
                        warn!(
                            "Account {} is already locked by another request {}, dropping request {}",
                            hex::encode(req.account.to_raw_bytes()),
                            concurrent_req,
                            req.id
                        );
                        return Control::Drop(task_id);
                    }
                    batch.add_order(order, payout);
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
                    hex::encode(req.account.to_raw_bytes()),
                    req.id
                );
                return Control::Drop(task_id);
            }
        }
        Control::Next
    }

    async fn execute(&mut self) -> Result<ExecutionResult<TaskId, SignedTxBuilder>, ()> {
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
            let mut sorted_inputs: Vec<_> = batch
                .orders
                .into_iter()
                .map(
                    |OrderWithPayout {
                         order:
                             Bundled(
                        HarvestOrder {
                            account,
                            owner_stake_credential,
                            ..
                        },
                        tx_out,
                    ),
                         payout,
                     }| {
                        accounts.push((account, owner_stake_credential, payout, tx_out.0.value().coin));
                        let harvest_order_input =
                            SingleInputBuilder::new(TransactionInput::from(tx_out.1), tx_out.0)
                                .plutus_script_inline_datum(
                                    harvest_order_witness.clone(),
                                    RequiredSigners::from(vec![]),
                                )
                                .unwrap();

                        (harvest_order_input, ex_units.clone())
                    },
                )
                .collect();

            let Bundled(_, FinalizedTxOut(bw_tx_out, output_ref)) = batch.buffer_wallet;

            let buffer_wallet_input =
                SingleInputBuilder::new(TransactionInput::from(output_ref), bw_tx_out.clone())
                    .native_script(
                        buffer_wallet_script.clone(),
                        NativeScriptWitnessInfo::num_signatures(2),
                    )
                    .unwrap();
            sorted_inputs.push((buffer_wallet_input, None));

            sorted_inputs.sort_by_key(|(input, _)| input.input.clone());

            // Outputs
            let network_id = self.ctx.select::<NetworkId>();

            let mut bw_out = bw_tx_out;

            let splash_asset_name = AssetName::from_utf8(SPLASH_NAME.into());
            let splash_policy = self.ctx.select::<SplashPolicy>().0;
            let splash_asset_class = AssetClass::Token(Token(splash_policy, splash_asset_name));

            assert!(bw_out
                .value_mut()
                .checked_sub(&make_splash_value(splash_asset_class, batch.total_payout))
                .is_ok());

            let buffer_wallet_output = SingleOutputBuilderResult::new(bw_out);

            let mut outputs = vec![buffer_wallet_output];

            for (key_hash, owner_stake_credential, payout, coin) in accounts {
                let payment_cred = Credential::new_pub_key(key_hash);
                let user_addr = if let Some(stake_cred) = owner_stake_credential {
                    BaseAddress::new(network_id.into(), payment_cred, stake_cred).to_address()
                } else {
                    EnterpriseAddress::new(network_id.into(), payment_cred).to_address()
                };

                let mut user_value = Value::from(coin);
                // The TX fee is shared equally among all accounts receiving a payout.
                let reduction = HARVESTING_TX_ASSUMED_BASE_FEE / num_payouts;
                assert!(user_value.coin > reduction);
                user_value.coin -= reduction;
                user_value.add_unsafe(splash_asset_class, payout);

                let user_payout_output = TransactionOutputBuilder::new()
                    .with_address(user_addr)
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
            let output = tx_builder
                .build(ChangeSelectionAlgo::Default, &operator_address)
                .unwrap();

            let tx_body = output.body();
            let tx_hash = <[u8; 32]>::from(hash_transaction_canonical(&tx_body));
            let task_id = TaskId::from(tx_hash);

            Ok(ExecutionResult {
                executed_tasks: vec![task_id],
                output,
            })
        } else {
            Err(())
        }
    }
}

fn reward_amount(share_bps: u64, interval_emission: u64) -> u64 {
    (share_bps * interval_emission) / 10_000
}

pub struct BufferingFlow<GaugeId, StateId, Bearer, Tx, OnChainIndex> {
    onchain_index: OnChainIndex,
    batch: Option<BufferingBatch<GaugeId, StateId, Bearer>>,
    pd: PhantomData<Tx>,
}

#[async_trait]
impl<GaugeId, StateId, Bearer, Tx, OnChainIndex> BatchExecutor<TaskId, GaugeBuffering<GaugeId>, Tx, ()>
    for BufferingFlow<GaugeId, StateId, Bearer, Tx, OnChainIndex>
where
    GaugeId: Display + Copy + Send + 'static,
    StateId: Send + 'static,
    Bearer: Send,
    Tx: Send,
    OnChainIndex: GaugeIndex<GaugeId, StateId, Bearer> + BufferWalletIndex<StateId, Bearer> + Send,
{
    async fn feed(&mut self, task_id: TaskId, task: GaugeBuffering<GaugeId>) -> Control<TaskId> {
        let batch = if let Some(ref mut batch) = self.batch {
            batch
        } else {
            if let Some(bw) = self.onchain_index.get_buffer_wallet().await {
                self.batch.insert(BufferingBatch::new(bw))
            } else {
                error!("No buffer wallet found");
                return Control::Stop;
            }
        };
        if let Some(gauge) = self.onchain_index.get_gauge(task.gauge_id).await {
            batch.add_gauge(gauge);
        } else {
            warn!("Gauge {} not found, dropping task {}", task.gauge_id, task_id);
            return Control::Drop(task_id);
        }
        Control::Next
    }

    async fn execute(&mut self) -> Result<ExecutionResult<TaskId, Tx>, ()> {
        todo!("DEX-891")
    }
}

fn make_splash_value(splash_asset_class: AssetClass, amount: u64) -> Value {
    let mut splash_tokens_value = Value::zero();
    splash_tokens_value.add_unsafe(splash_asset_class, amount);
    splash_tokens_value
}

pub enum Flow<GaugeId, StateId, Bearer, Tx, Ctx, OnChainIndex, PositionIndex, Emission> {
    Harvesting(HarvestingFlow<StateId, Bearer, Tx, Ctx, PositionIndex, OnChainIndex, Emission>),
    Buffering(BufferingFlow<GaugeId, StateId, Bearer, Tx, OnChainIndex>),
}

pub struct Executor<GaugeId, StateId, Bearer, Tx, Ctx, TxErr, PositionIndex, OnChainIndex, TxSubmit, Emission>
{
    position_index: PositionIndex,
    onchain_index: OnChainIndex,
    tx_submit: TxSubmit,
    emission: Emission,
    flow: Option<Flow<GaugeId, StateId, Bearer, Tx, Ctx, OnChainIndex, PositionIndex, Emission>>,
    ctx: Ctx,
    pd: PhantomData<(Tx, TxErr)>,
}

#[async_trait]
impl<GaugeId, StateId, Bearer, Tx, Ctx, TxErr, PositionIndex, OnChainIndex, TxSubmit, Emiss>
    BatchExecutor<TaskId, Task<GaugeId, StateId>, (), ()>
    for Executor<GaugeId, StateId, Bearer, Tx, Ctx, TxErr, PositionIndex, OnChainIndex, TxSubmit, Emiss>
where
    GaugeId: Copy + Send + Display + 'static,
    StateId: Send + Sync + Display + 'static,
    Bearer: Send,
    Tx: Send,
    TxErr: Send,
    PositionIndex: Positions<StateId> + Clone + Send,
    OnChainIndex: GaugeIndex<GaugeId, StateId, Bearer>
        + OrderIndex<StateId, Bearer>
        + BufferWalletIndex<StateId, Bearer>
        + Clone
        + Send,
    TxSubmit: Clone + Network<Tx, TxErr> + Send,
    Emiss: Emission + Clone + Send,
    HarvestingFlow<StateId, Bearer, Tx, Ctx, PositionIndex, OnChainIndex, Emiss>:
        BatchExecutor<TaskId, Harvesting<StateId>, Tx, ()>,
    Ctx: Send
        + Clone
        + Has<BufferWalletScript>
        + Has<HarvestOrderScriptHash>
        + Has<HarvestOrderRefScriptOutput>
        + Has<NetworkId>,
{
    async fn feed(&mut self, task_id: TaskId, task: Task<GaugeId, StateId>) -> Control<TaskId> {
        let flow = match self.flow {
            None => match task {
                Task::GaugeBuffering(_) => self.flow.insert(Flow::Buffering(BufferingFlow {
                    onchain_index: self.onchain_index.clone(),
                    batch: None,
                    pd: PhantomData,
                })),
                Task::Harvesting(_) => self.flow.insert(Flow::Harvesting(HarvestingFlow {
                    position_index: self.position_index.clone(),
                    onchain_index: self.onchain_index.clone(),
                    emission: self.emission.clone(),
                    batch: None,
                    ctx: self.ctx.clone(),
                    pd: PhantomData,
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
        match self.flow.take() {
            None => Err(()),
            Some(flow) => {
                let ExecutionResult {
                    executed_tasks,
                    output,
                } = match flow {
                    Flow::Harvesting(mut hf) => hf.execute().await?,
                    Flow::Buffering(mut bf) => bf.execute().await?,
                };
                match self.tx_submit.submit_tx(output).await {
                    Ok(_) => {
                        //todo!("DEX-892 index transaction io as unconfirmed changes to entities' states")
                        Ok(ExecutionResult {
                            executed_tasks,
                            output: (),
                        })
                    }
                    Err(_) => {
                        //todo!("DEX-892 invalidate 'spent' states in the index")
                        Err(())
                    }
                }
            }
        }
    }
}
