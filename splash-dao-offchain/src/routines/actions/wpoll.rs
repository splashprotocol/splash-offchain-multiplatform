use std::ops::DerefMut;
use std::time::{SystemTime, UNIX_EPOCH};

use cml_chain::address::Address;
use cml_chain::assets::AssetBundle;
use cml_chain::builders::input_builder::{InputBuilderResult, SingleInputBuilder};
use cml_chain::builders::mint_builder::SingleMintBuilder;
use cml_chain::builders::output_builder::{SingleOutputBuilderResult, TransactionOutputBuilder};
use cml_chain::builders::redeemer_builder::RedeemerWitnessKey;
use cml_chain::builders::tx_builder::{ChangeSelectionAlgo, SignedTxBuilder, TransactionUnspentOutput};
use cml_chain::builders::withdrawal_builder::SingleWithdrawalBuilder;
use cml_chain::builders::witness_builder::{PartialPlutusWitness, PlutusScriptWitness};
use cml_chain::certs::Credential;
use cml_chain::min_ada::min_ada_required;
use cml_chain::plutus::{PlutusScript, PlutusV2Script, PlutusV3Script, RedeemerTag};
use cml_chain::transaction::{TransactionInput, TransactionOutput};
use cml_chain::utils::BigInteger;
use cml_chain::{Deserialize, OrderedHashMap, RequiredSigners};
use cml_crypto::{Ed25519Signature, RawBytesEncoding};
use log::trace;
use spectrum_cardano_lib::types::TryFromPData;
use spectrum_cardano_lib::value::ValueExtension;
use spectrum_offchain::domain::event::{Predicted, Traced};
use spectrum_offchain_cardano::deployment::{DeployedScriptInfo, DeployedValidator};

use bloom_offchain::execution_engine::bundled::Bundled;
use spectrum_cardano_lib::collateral::Collateral;
use spectrum_cardano_lib::hash::hash_transaction_canonical;
use spectrum_cardano_lib::plutus_data::IntoPlutusData;
use spectrum_cardano_lib::protocol_params::{constant_tx_builder, COINS_PER_UTXO_BYTE};
use spectrum_cardano_lib::transaction::TransactionOutputExtension;
use spectrum_cardano_lib::{AssetClass, AssetName, NetworkId, OutputRef, Token};
use spectrum_offchain::domain::Has;
use spectrum_offchain::ledger::IntoLedger;

use crate::constants::fee_deltas::{
    CREATE_WPOLL_FEE_DELTA, ELIMINATE_WPOLL_FEE_DELTA, VOTING_ESCROW_VOTING_FEE, WPOLL_VOTE_ORDER_FEE_DELTA,
};
use crate::constants::{CREATE_WPOLL_MINIMUM_FUNDING, ELIMINATE_WPOLL_MINIMUM_FUNDING, SPLASH_NAME};
use crate::create_change_output::{ChangeOutputCreator, CreateChangeOutput};
use crate::deployment::{DaoScriptData, ProtocolValidator};
use crate::entities::offchain::{compute_witness_message, WPollVoteOffChainOrder};
use crate::entities::onchain::funding_box::{FundingBox, FundingBoxId};
use crate::entities::onchain::inflation_box::{unsafe_update_ibox_state, InflationBoxSnapshot};
use crate::entities::onchain::permission_manager::PermManagerSnapshot;
use crate::entities::onchain::poll_factory::{
    unsafe_update_factory_state, FactoryRedeemer, PollFactoryAction, PollFactorySnapshot,
};
use crate::entities::onchain::voting_escrow::{
    self, Lock, Owner, VotingEscrowAction, VotingEscrowAuthorizedAction, VotingEscrowConfig,
    VotingEscrowSnapshot,
};
use crate::entities::onchain::weighting_poll::{
    self, unsafe_update_wp_state, MintAction, WeightingPollSnapshot,
};
use crate::entities::onchain::wpoll_vote_order::{
    WPollVoteAction, WPollVoteOnchainOrder, WPollVoteOrderBundle,
};
use crate::entities::Snapshot;
use crate::protocol_config::{GTAuthPolicy, OperatorCreds, PermManagerAuthPolicy, Reward, SplashPolicy};
use crate::routines::actions::{
    AvailableFundingBoxes, BlueprintEstimates, DaoTxBlueprint, FundingBoxChanges, Slot, WitnessError,
};
use crate::routines::TimedOutputRef;
use crate::time::epoch_end;
use crate::util::set_min_ada;
use crate::GenesisEpochStartTime;

use super::{
    compute_epoch_asset_name, select_funding_boxes, CardanoInflationActions, ExecuteOrderError, WPollActions,
    TX_TTL_SLOT,
};

#[async_trait::async_trait]
impl<Ctx> WPollActions<TransactionOutput> for CardanoInflationActions<Ctx>
where
    Ctx: Send
        + Sync
        + Clone
        + Has<DeployedValidator<{ ProtocolValidator::Inflation as u8 }>>
        + Has<DeployedValidator<{ ProtocolValidator::VotingEscrow as u8 }>>
        + Has<DeployedValidator<{ ProtocolValidator::WpFactory as u8 }>>
        + Has<DeployedValidator<{ ProtocolValidator::MintWpAuthPolicy as u8 }>>
        + Has<DeployedValidator<{ ProtocolValidator::WPollVoteOrder as u8 }>>
        + Has<DeployedValidator<{ ProtocolValidator::WeightingPower as u8 }>>
        + Has<SplashPolicy>
        + Has<OperatorCreds>
        + Has<GenesisEpochStartTime>
        + Has<PermManagerAuthPolicy>
        + Has<GTAuthPolicy>
        + Has<NetworkId>
        + Has<Collateral>
        + Has<Reward>,
{
    async fn create_wpoll(
        &self,
        Bundled(inflation_box, inflation_box_in): Bundled<InflationBoxSnapshot, TransactionOutput>,
        Bundled(factory, factory_in): Bundled<PollFactorySnapshot, TransactionOutput>,
        current_slot: Slot,
        funding_boxes: AvailableFundingBoxes,
    ) -> (
        SignedTxBuilder,
        Traced<Predicted<Bundled<InflationBoxSnapshot, TransactionOutput>>>,
        Traced<Predicted<Bundled<PollFactorySnapshot, TransactionOutput>>>,
        Traced<Predicted<Bundled<WeightingPollSnapshot, TransactionOutput>>>,
        FundingBoxChanges,
    ) {
        let mut change_output_creator = ChangeOutputCreator::default();
        let mut tx_builder = constant_tx_builder();

        // Set TX validity range
        tx_builder.set_validity_start_interval(current_slot.0);
        tx_builder.set_ttl(current_slot.0 + TX_TTL_SLOT);

        let inflation_deployed_validator = self
            .ctx
            .select::<DeployedValidator<{ ProtocolValidator::Inflation as u8 }>>();

        let inflation_script_hash = inflation_deployed_validator.hash;
        let inflation_script = PartialPlutusWitness::new(
            PlutusScriptWitness::Ref(inflation_script_hash),
            cml_chain::plutus::PlutusData::Integer(BigInteger::from(0)),
        );

        let inflation_input = SingleInputBuilder::new(
            TransactionInput::from(inflation_box.version().output_ref),
            inflation_box_in.clone(),
        )
        .plutus_script_inline_datum(inflation_script, RequiredSigners::from(vec![]))
        .unwrap();

        tx_builder.add_reference_input(inflation_deployed_validator.reference_utxo);

        let (next_inflation_box, emission_rate) = inflation_box.get().release_next_tranche();
        let mut inflation_box_out = inflation_box_in.clone();
        if let Some(data_mut) = inflation_box_out.data_mut() {
            // Following unwrap is safe due to the `.release_next_trache()` call above.
            unsafe_update_ibox_state(data_mut, next_inflation_box.last_processed_epoch.unwrap() + 1);
        }
        let splash_policy = self.ctx.select::<SplashPolicy>().0;
        let splash_asset_class =
            AssetClass::Token(Token(splash_policy, AssetName::from_utf8(SPLASH_NAME.into())));
        inflation_box_out.sub_asset(splash_asset_class, emission_rate.untag());
        set_min_ada(&mut inflation_box_out);
        let inflation_output = SingleOutputBuilderResult::new(inflation_box_out.clone());

        // WP factory

        let wp_factory_script_hash = self
            .ctx
            .select::<DeployedValidator<{ ProtocolValidator::WpFactory as u8 }>>()
            .hash;

        let factory_redeemer = FactoryRedeemer {
            successor_ix: 2,
            action: PollFactoryAction::CreatePoll,
        };
        let wp_factory_script = PartialPlutusWitness::new(
            PlutusScriptWitness::Ref(wp_factory_script_hash),
            factory_redeemer.into_pd(),
        );

        let wp_factory_input = SingleInputBuilder::new(
            TransactionInput::from(factory.version().output_ref),
            factory_in.clone(),
        )
        .plutus_script_inline_datum(wp_factory_script, RequiredSigners::from(vec![]))
        .unwrap();

        tx_builder.add_reference_input(
            self.ctx
                .select::<DeployedValidator<{ ProtocolValidator::WpFactory as u8 }>>()
                .reference_utxo,
        );

        let (next_factory, fresh_wpoll) = factory.unwrap().next_weighting_poll(emission_rate);
        let mut factory_out = factory_in;
        if let Some(data_mut) = factory_out.data_mut() {
            unsafe_update_factory_state(data_mut, next_factory.last_poll_epoch.unwrap());
        }

        let (input_results, funding_boxes_to_spend) =
            select_funding_boxes(CREATE_WPOLL_MINIMUM_FUNDING, vec![], funding_boxes, &self.ctx);

        let mut unsorted_inputs: Vec<_> = input_results
            .into_iter()
            .map(|input| (CreateWPollInputType::Funding, input))
            .collect();

        unsorted_inputs.extend([
            (CreateWPollInputType::Inflation, inflation_input),
            (CreateWPollInputType::WPFactory, wp_factory_input),
        ]);
        let (
            input_results,
            MintWPAuthTokensIx {
                factory_in_ix,
                inflation_box_in_ix,
            },
        ) = sort_create_wp_poll_tx_inputs(unsorted_inputs);

        trace!(
            "create_wpoll: factory_in_ix: {}, inflation_box_in_ix: {}",
            factory_in_ix,
            inflation_box_in_ix
        );

        for input in input_results {
            change_output_creator.add_input(&input);
            tx_builder.add_input(input).unwrap();
        }

        let inflation_ex_units = DaoScriptData::global().inflation.ex_units.clone();
        let wp_factory_ex_units = DaoScriptData::global().wp_factory.ex_units.clone();

        if inflation_box_in_ix < factory_in_ix {
            tx_builder.set_exunits(
                RedeemerWitnessKey::new(RedeemerTag::Spend, inflation_box_in_ix),
                inflation_ex_units,
            );
            tx_builder.set_exunits(
                RedeemerWitnessKey::new(RedeemerTag::Spend, factory_in_ix),
                wp_factory_ex_units,
            );
        } else {
            tx_builder.set_exunits(
                RedeemerWitnessKey::new(RedeemerTag::Spend, factory_in_ix),
                wp_factory_ex_units,
            );
            tx_builder.set_exunits(
                RedeemerWitnessKey::new(RedeemerTag::Spend, inflation_box_in_ix),
                inflation_ex_units,
            );
        }

        let mint_action = MintAction::MintAuthToken {
            factory_in_ix: factory_in_ix as u32,
            inflation_box_in_ix: inflation_box_in_ix as u32,
        };

        let wp_auth_deployed_validator = self
            .ctx
            .select::<DeployedValidator<{ ProtocolValidator::MintWpAuthPolicy as u8 }>>();
        let wp_auth_policy = wp_auth_deployed_validator.hash;
        let mint_wp_auth_token_witness =
            PartialPlutusWitness::new(PlutusScriptWitness::Ref(wp_auth_policy), mint_action.into_pd());
        let OperatorCreds(_operator_pkh, _operator_addr) = self.ctx.select::<OperatorCreds>();

        // Compute index_tn(epoch), where `epoch` is the current epoch
        let asset = compute_epoch_asset_name(
            inflation_box
                .get()
                .last_processed_epoch
                .map(|epoch| epoch + 1)
                .unwrap_or(0),
        );
        let wp_auth_minting_policy = SingleMintBuilder::new_single_asset(asset.clone(), 1)
            .plutus_script(mint_wp_auth_token_witness, RequiredSigners::from(vec![]));
        tx_builder.add_reference_input(wp_auth_deployed_validator.reference_utxo);
        tx_builder.add_mint(wp_auth_minting_policy).unwrap();
        tx_builder.set_exunits(
            RedeemerWitnessKey::new(RedeemerTag::Mint, 0),
            DaoScriptData::global().mint_wp_auth_token.mint_ex_units.clone(),
        );

        // Contracts require that weighting_poll output resides at index 1.
        let mut wpoll_out = fresh_wpoll.clone().into_ledger(self.ctx.clone());
        // Add wp_auth_token to this output.
        let asset_pair = OrderedHashMap::from_iter(vec![(asset, 1)]);
        let ord_hash_map = OrderedHashMap::from_iter(vec![(wp_auth_policy, asset_pair)]);
        match &mut wpoll_out {
            TransactionOutput::AlonzoFormatTxOut(tx_out) => {
                let multiasset = tx_out
                    .amount
                    .multiasset
                    .checked_add(&AssetBundle::from(ord_hash_map))
                    .unwrap();
                tx_out.amount.multiasset = multiasset;
            }

            TransactionOutput::ConwayFormatTxOut(tx_out) => {
                let multiasset = tx_out
                    .amount
                    .multiasset
                    .checked_add(&AssetBundle::from(ord_hash_map))
                    .unwrap();
                tx_out.amount.multiasset = multiasset;
            }
        }

        change_output_creator.add_output(&inflation_output);
        tx_builder.add_output(inflation_output).unwrap();

        set_min_ada(&mut wpoll_out);
        let weighting_poll_output = SingleOutputBuilderResult::new(wpoll_out.clone());
        change_output_creator.add_output(&weighting_poll_output);
        tx_builder.add_output(weighting_poll_output).unwrap();

        set_min_ada(&mut factory_out);
        let factory_output = SingleOutputBuilderResult::new(factory_out.clone());
        change_output_creator.add_output(&factory_output);
        tx_builder.add_output(factory_output).unwrap();

        // Set Governance Proxy witness script
        let OperatorCreds(_, operator_address) = self.ctx.select::<OperatorCreds>();

        tx_builder
            .add_collateral(InputBuilderResult::from(self.ctx.select::<Collateral>()))
            .unwrap();

        let estimated_tx_fee = tx_builder.min_fee(true).unwrap();
        let actual_fee = estimated_tx_fee + CREATE_WPOLL_FEE_DELTA;
        let change_output = change_output_creator.create_change_output(actual_fee, operator_address.clone());
        tx_builder.add_output(change_output).unwrap();

        // Build tx, change is execution fee.
        let signed_tx_builder = tx_builder
            .build(ChangeSelectionAlgo::Default, &operator_address)
            .unwrap();
        let tx_body = signed_tx_builder.body();

        let tx_hash = hash_transaction_canonical(&tx_body);

        let add_slot = |output_ref| TimedOutputRef {
            output_ref,
            slot: current_slot,
        };

        // Extract newly-created funding-boxes, which are change boxes created by the TX builder.
        let created_funding_boxes: Vec<_> = tx_body
            .outputs
            .iter()
            .enumerate()
            .skip(3)
            .map(|(ix, output)| {
                assert_eq!(*output.address(), operator_address);
                let output_ref = OutputRef::new(tx_hash, ix as u64);
                let value = output.value().clone();
                let funding_box = FundingBox {
                    value,
                    id: FundingBoxId::from(output_ref),
                };
                Predicted(funding_box)
            })
            .collect();

        let spent_predicted = funding_boxes_to_spend
            .predicted
            .into_iter()
            .map(|f| f.id)
            .collect();
        let spent_confirmed = funding_boxes_to_spend
            .confirmed
            .into_iter()
            .map(|f| f.id)
            .collect();

        let funding_box_changes = FundingBoxChanges {
            spent_predicted,
            spent_confirmed,
            created: created_funding_boxes,
        };

        let next_ib_version = add_slot(OutputRef::new(tx_hash, 0));
        let next_traced_ibox = Traced::new(
            Predicted(Bundled(
                Snapshot::new(next_inflation_box, next_ib_version),
                inflation_box_out.clone(),
            )),
            None,
        );
        let fresh_wpoll_version = add_slot(OutputRef::new(tx_hash, 1));
        let fresh_wpoll = Traced::new(
            Predicted(Bundled(
                Snapshot::new(fresh_wpoll, fresh_wpoll_version),
                wpoll_out,
            )),
            None,
        );
        let next_factory_version = add_slot(OutputRef::new(tx_hash, 2));
        let next_traced_factory = Traced::new(
            Predicted(Bundled(
                Snapshot::new(next_factory, next_factory_version),
                factory_out.clone(),
            )),
            None,
        );
        (
            signed_tx_builder,
            next_traced_ibox,
            next_traced_factory,
            fresh_wpoll,
            funding_box_changes,
        )
    }

    async fn eliminate_wpoll(
        &self,
        Bundled(weighting_poll, weighting_poll_in): Bundled<WeightingPollSnapshot, TransactionOutput>,
        Bundled(perm_manager, perm_manager_in): Bundled<PermManagerSnapshot, TransactionOutput>,
        funding_boxes: AvailableFundingBoxes,
        current_slot: Slot,
    ) -> (SignedTxBuilder, FundingBoxChanges) {
        let mut tx_builder = constant_tx_builder();
        tx_builder.set_validity_start_interval(current_slot.0);
        tx_builder.set_ttl(current_slot.0 + TX_TTL_SLOT);

        let mint_wp_auth_deployed_validator = self
            .ctx
            .select::<DeployedValidator<{ ProtocolValidator::MintWpAuthPolicy as u8 }>>();

        let weighting_power_deployed_validator = self
            .ctx
            .select::<DeployedValidator<{ ProtocolValidator::WeightingPower as u8 }>>();

        let mint_weighting_power_ref_input = weighting_power_deployed_validator.reference_utxo;
        let wpoll_auth_ref_input = mint_wp_auth_deployed_validator.reference_utxo;
        let wpoll_script_hash = mint_wp_auth_deployed_validator.hash;

        enum T {
            PermManager,
            Other,
        }

        let perm_manager_unspent_input = TransactionUnspentOutput::new(
            TransactionInput::from(perm_manager.version().output_ref),
            perm_manager_in.clone(),
        );

        // Need to determine the index of `perm_manager` within `reference_inputs`
        let mut indexed_ref_inputs = vec![
            (perm_manager_unspent_input, T::PermManager),
            (mint_weighting_power_ref_input, T::Other),
            (wpoll_auth_ref_input, T::Other),
        ];
        indexed_ref_inputs.sort_by_key(|(input, _)| input.input.clone());
        let perm_manager_input_ix = indexed_ref_inputs
            .iter()
            .position(|(_, typ)| matches!(typ, T::PermManager))
            .unwrap() as u32;

        for (ref_input, _) in indexed_ref_inputs {
            tx_builder.add_reference_input(ref_input);
        }

        let redeemer = weighting_poll::PollAction::Destroy {
            perm_manager_input_ix,
        };
        let weighting_poll_script =
            PartialPlutusWitness::new(PlutusScriptWitness::Ref(wpoll_script_hash), redeemer.into_pd());

        let weighting_poll_input = SingleInputBuilder::new(
            TransactionInput::from(weighting_poll.version().output_ref),
            weighting_poll_in.clone(),
        )
        .plutus_script_inline_datum(weighting_poll_script, RequiredSigners::from(vec![]))
        .unwrap();

        let mut output_value = match weighting_poll_in {
            TransactionOutput::AlonzoFormatTxOut(tx) => tx.amount.clone(),
            TransactionOutput::ConwayFormatTxOut(tx) => tx.amount.clone(),
        };

        let (input_results, funding_boxes_to_spend) =
            select_funding_boxes(ELIMINATE_WPOLL_MINIMUM_FUNDING, vec![], funding_boxes, &self.ctx);

        let mut inputs: Vec<_> = input_results
            .into_iter()
            .map(|input| (EliminateWPollInputType::Funding, input))
            .collect();

        inputs.push((EliminateWPollInputType::WPoll, weighting_poll_input));
        inputs.sort_by_key(|input| input.1.input.clone());

        let wpoll_ix = inputs
            .iter()
            .position(|(input_type, _)| matches!(input_type, EliminateWPollInputType::WPoll))
            .unwrap() as u64;
        trace!("`eliminate_wpoll` input: wpoll_ix: {}", wpoll_ix);

        let mut change_output_creator = ChangeOutputCreator::default();
        for (_, input) in inputs {
            change_output_creator.add_input(&input);
            tx_builder.add_input(input).unwrap();
        }

        // Burn weighting_poll's token -------------------------------------------------------------
        let mut names = output_value
            .multiasset
            .deref_mut()
            .remove(&wpoll_script_hash)
            .unwrap();
        assert_eq!(names.len(), 1);
        let (name, qty) = names.pop_front().unwrap();
        assert_eq!(qty, 1);

        change_output_creator.burn_token(crate::create_change_output::Token {
            policy_id: wpoll_script_hash,
            asset_name: name.clone(),
            quantity: 1,
        });

        let mint_action = MintAction::BurnAuthToken;
        let mint_wp_auth_token_witness =
            PartialPlutusWitness::new(PlutusScriptWitness::Ref(wpoll_script_hash), mint_action.into_pd());
        let wp_auth_minting_policy = SingleMintBuilder::new_single_asset(name.clone(), -1)
            .plutus_script(mint_wp_auth_token_witness, RequiredSigners::from(vec![]));
        tx_builder.add_mint(wp_auth_minting_policy).unwrap();

        // Burn weighting_power tokens -------------------------------------------------------------

        let dsd = DaoScriptData::global();
        tx_builder.set_exunits(
            RedeemerWitnessKey::new(RedeemerTag::Mint, 0),
            dsd.mint_wp_auth_token.burn_ex_units.clone(),
        );

        let mint_weighting_power_policy = weighting_power_deployed_validator.hash;

        // If there exists weighting power, burn it.
        if let Some(weighting_power) = weighting_poll.get().weighting_power {
            let mut names = output_value
                .multiasset
                .deref_mut()
                .remove(&mint_weighting_power_policy)
                .unwrap();
            assert_eq!(names.len(), 1);
            let (mint_weighting_power_token_name, qty) = names.pop_front().unwrap();
            assert_eq!(qty, weighting_power);
            assert_eq!(mint_weighting_power_token_name, name);

            let mint_action = voting_escrow::MintAction::Burn;
            let mint_wp_auth_token_witness = PartialPlutusWitness::new(
                PlutusScriptWitness::Ref(mint_weighting_power_policy),
                mint_action.into_pd(),
            );
            let mint_weighting_power_builder_result =
                SingleMintBuilder::new_single_asset(name.clone(), -(weighting_power as i64))
                    .plutus_script(mint_wp_auth_token_witness, RequiredSigners::from(vec![]));
            tx_builder.add_mint(mint_weighting_power_builder_result).unwrap();

            change_output_creator.burn_token(crate::create_change_output::Token {
                policy_id: mint_weighting_power_policy,
                asset_name: mint_weighting_power_token_name,
                quantity: weighting_power,
            });
            tx_builder.set_exunits(
                RedeemerWitnessKey::new(RedeemerTag::Mint, 1),
                dsd.mint_weighting_power.burn_ex_units.clone(),
            );
        }

        let OperatorCreds(_, operator_addr) = self.ctx.select::<OperatorCreds>();
        let output = TransactionOutputBuilder::new()
            .with_address(operator_addr.clone())
            .next()
            .unwrap()
            .with_value(output_value)
            .build()
            .unwrap();
        change_output_creator.add_output(&output);
        tx_builder.add_output(output).unwrap();
        tx_builder.set_exunits(
            RedeemerWitnessKey::new(RedeemerTag::Spend, wpoll_ix),
            DaoScriptData::global().mint_wp_auth_token.mint_ex_units.clone(),
        );

        let estimated_tx_fee = tx_builder.min_fee(true).unwrap() + ELIMINATE_WPOLL_FEE_DELTA;
        let change_output =
            change_output_creator.create_change_output(estimated_tx_fee, operator_addr.clone());
        tx_builder.add_output(change_output).unwrap();
        tx_builder.set_fee(estimated_tx_fee);

        // Add operator as signatory
        let OperatorCreds(operator_pkh, _) = self.ctx.select::<OperatorCreds>();
        tx_builder.add_required_signer(operator_pkh);

        tx_builder
            .add_collateral(InputBuilderResult::from(self.ctx.select::<Collateral>()))
            .unwrap();

        let execution_fee_address: Address = self.ctx.select::<Reward>().0.clone().into();
        let signed_tx_builder = tx_builder
            .build(ChangeSelectionAlgo::Default, &execution_fee_address)
            .unwrap();
        let tx_body = signed_tx_builder.body();

        let tx_hash = hash_transaction_canonical(&tx_body);

        // Extract newly-created funding-boxes, which are change boxes created by the TX builder.
        let created_funding_boxes: Vec<_> = tx_body
            .outputs
            .iter()
            .enumerate()
            .skip(1)
            .map(|(ix, output)| {
                assert_eq!(*output.address(), operator_addr);
                let output_ref = OutputRef::new(tx_hash, ix as u64);
                let value = output.value().clone();
                let funding_box = FundingBox {
                    value,
                    id: FundingBoxId::from(output_ref),
                };
                Predicted(funding_box)
            })
            .collect();

        let spent_predicted = funding_boxes_to_spend
            .predicted
            .into_iter()
            .map(|f| f.id)
            .collect();
        let spent_confirmed = funding_boxes_to_spend
            .confirmed
            .into_iter()
            .map(|f| f.id)
            .collect();

        let funding_box_changes = FundingBoxChanges {
            spent_predicted,
            spent_confirmed,
            created: created_funding_boxes,
        };

        (signed_tx_builder, funding_box_changes)
    }

    async fn execute_order(
        &self,
        Bundled(weighting_poll, weighting_poll_in): Bundled<WeightingPollSnapshot, TransactionOutput>,
        Bundled(voting_escrow, ve_box_in): Bundled<VotingEscrowSnapshot, TransactionOutput>,
        onchain_order: WPollVoteOrderBundle<TransactionOutput>,
        current_slot: Slot,
    ) -> Result<
        (
            SignedTxBuilder,
            Traced<Predicted<Bundled<WeightingPollSnapshot, TransactionOutput>>>,
            Traced<Predicted<Bundled<VotingEscrowSnapshot, TransactionOutput>>>,
        ),
        ExecuteOrderError,
    > {
        // Voting escrow ---------------------------------------------------------------------------
        let mut voting_escrow_out = ve_box_in.clone();
        let data_mut = voting_escrow_out.data_mut().unwrap();
        let mut ve_state @ VotingEscrowConfig {
            owner,
            last_wp_epoch,
            version,
            locked_until,
            ..
        } = VotingEscrowConfig::try_from_pd(data_mut.clone()).unwrap();

        let order_version = onchain_order.order.datum.ve_state.version;
        let metadata = onchain_order.order.metadata;
        // Verify that witness is authorized by the owner.
        if let Owner::PubKey(bytes) = owner {
            let pk = cml_crypto::PublicKey::from_raw_bytes(&bytes)
                .map_err(|_| ExecuteOrderError::Other("Can't extrat PublicKey from bytes".into()))?;
            let signature = Ed25519Signature::from_raw_bytes(&metadata.signature)
                .map_err(|_| ExecuteOrderError::Other("Can't extract Ed25519Signature from bytes".into()))?;
            println!("execute_order hash: {}", metadata.witness_script_hash.to_hex());
            use cml_chain::Serialize;
            println!(
                " order datum: {}",
                hex::encode(onchain_order.order.datum.clone().into_pd().to_cbor_bytes())
            );
            println!("version: {}", order_version);
            let message = compute_witness_message(
                metadata.witness_script_hash,
                &onchain_order.order.datum.clone().into_pd(),
                metadata.version,
            );
            println!("message: {}", hex::encode(&message));
            // Message with both prefix and postfix bytes.
            let full_message: Vec<u8> = metadata
                .prefix_bytes
                .iter()
                .chain(message.iter())
                .chain(metadata.postfix_bytes.iter())
                .cloned()
                .collect();
            println!("pre/post-fixed message: {}", hex::encode(&full_message));
            if !pk.verify(&full_message, &signature) {
                return Err(ExecuteOrderError::Witness(WitnessError::OwnerAuthFailure));
            }
        }

        let new_wp_epoch = weighting_poll.get().epoch;

        // Check `ve_is_eligible_to_vote_in_this_epoch` predicate from `mint_weighting_power`.
        if last_wp_epoch >= new_wp_epoch as i32 {
            return Err(ExecuteOrderError::Witness(
                WitnessError::VotingEscrowIneligibleToVote {
                    last_wp_epoch,
                    current_epoch: new_wp_epoch as i32,
                },
            ));
        }

        if version != order_version {
            return Err(ExecuteOrderError::Witness(
                WitnessError::VEVersionMismatchWithTXMetadata {
                    voting_escrow_input_version: version,
                    order_version,
                },
            ));
        }

        if let Lock::Def(until_millis) = locked_until {
            let gen_epoch_start = self.ctx.select::<GenesisEpochStartTime>();
            if until_millis < epoch_end(gen_epoch_start, new_wp_epoch) {
                return Err(ExecuteOrderError::LockTimeBeforeEpochEnd);
            }
        }

        // Sort inputs -----------------------------------------------------------------------------
        enum T {
            Order,
            VE,
            WPoll,
        }

        let prev_ve_version = voting_escrow.version();
        let prev_wp_version = weighting_poll.version();
        let order_output_ref = onchain_order.output_ref.output_ref;

        let dsd = DaoScriptData::global();

        let order_ex_units = Some(dsd.wpoll_vote_order.ex_units.clone());
        let ve_ex_units = Some(dsd.voting_escrow.ex_units.clone());
        let wp_ex_units = Some(dsd.mint_wp_auth_token.mint_ex_units.clone());

        let mut typed_inputs = vec![
            (T::Order, order_output_ref, order_ex_units),
            (T::VE, *prev_ve_version, ve_ex_units),
            (T::WPoll, prev_wp_version.output_ref, wp_ex_units),
        ];
        typed_inputs.sort_by(|(_, x, _), (_, y, _)| x.cmp(y));

        let order_input_ix = typed_inputs
            .iter()
            .position(|(t, _, _)| matches!(t, T::Order))
            .unwrap() as u32;
        let voting_escrow_input_ix = typed_inputs
            .iter()
            .position(|(t, _, _)| matches!(t, T::VE))
            .unwrap() as u32;
        let wpoll_input_ix = typed_inputs
            .iter()
            .position(|(t, _, _)| matches!(t, T::WPoll))
            .unwrap() as u32;

        trace!(
            "`execute_order` inputs: order_ix: {}, voting_escrow_ix: {}, wpoll_ix: {}",
            order_input_ix,
            voting_escrow_input_ix,
            wpoll_input_ix
        );

        let new_ve_version = voting_escrow.get().version + 1;
        ve_state.last_wp_epoch = new_wp_epoch as i32;
        ve_state.version = new_ve_version;

        // We create a new instance of the datum, because extracting the underlying
        // PlutusData::Constr to update fields in-place means we lose the original indefinite-array
        // containing the fields (CML uses definite-arrays by default).
        *data_mut = ve_state.into_pd();

        let mut next_ve = voting_escrow.get().clone();
        next_ve.last_wp_epoch = new_wp_epoch as i32;
        next_ve.version = new_ve_version;

        //let mut ve_amt = voting_escrow_out.amount().clone();
        //ve_amt.sub_unsafe(AssetClass::Native, VOTING_ESCROW_VOTING_FEE);
        //dbg!(&ve_amt);
        //voting_escrow_out.set_amount(ve_amt);

        let voting_escrow_order_deployed_validator = self
            .ctx
            .select::<DeployedValidator<{ ProtocolValidator::VotingEscrow as u8 }>>();

        let voting_escrow_ref_input = voting_escrow_order_deployed_validator.reference_utxo;
        let wpoll_auth_ref_input = self
            .ctx
            .select::<DeployedValidator<{ ProtocolValidator::MintWpAuthPolicy as u8 }>>()
            .reference_utxo;

        let weighting_power_deployed_validator = self
            .ctx
            .select::<DeployedValidator<{ ProtocolValidator::WeightingPower as u8 }>>();
        let weighting_power_ref_input = weighting_power_deployed_validator.reference_utxo;

        let wpoll_vote_order_deployed_validator = self
            .ctx
            .select::<DeployedValidator<{ ProtocolValidator::WPollVoteOrder as u8 }>>();
        let wpoll_vote_order_ref_input = wpoll_vote_order_deployed_validator.reference_utxo;

        let reference_inputs = vec![
            voting_escrow_ref_input,
            wpoll_auth_ref_input,
            weighting_power_ref_input,
            wpoll_vote_order_ref_input,
        ];

        // order input -----------------------------------------------------------------------------
        let order_script_hash = wpoll_vote_order_deployed_validator.hash;
        let order_action = WPollVoteAction::CastVote {
            voting_escrow_input_ix,
            wpoll_input_ix,
        };

        let order_witness = PartialPlutusWitness::new(
            PlutusScriptWitness::Ref(order_script_hash),
            order_action.clone().into_pd(),
        );

        let order_input_builder = SingleInputBuilder::new(
            TransactionInput::from(order_output_ref),
            onchain_order.bearer.clone(),
        )
        .plutus_script_inline_datum(order_witness, vec![].into())
        .unwrap();

        // voting_escrow input ---------------------------------------------------------------------
        let voting_escrow_script_hash = voting_escrow_order_deployed_validator.hash;

        let authorized_action = VotingEscrowAuthorizedAction {
            action: VotingEscrowAction::Governance,
            witness_ix: order_input_ix as u32,
            version,
            signature: metadata.signature,
            prefix_bytes: metadata.prefix_bytes,
            postfix_bytes: metadata.postfix_bytes,
        };
        let voting_escrow_witness = PartialPlutusWitness::new(
            PlutusScriptWitness::Ref(voting_escrow_script_hash),
            authorized_action.into_pd(),
        );

        let voting_escrow_input = SingleInputBuilder::new(
            TransactionInput::from(*voting_escrow.version()),
            ve_box_in.clone(),
        )
        .plutus_script_inline_datum(voting_escrow_witness, RequiredSigners::from(vec![]))
        .unwrap();

        // weighting_poll input --------------------------------------------------------------------
        let weighting_poll_script_hash = self
            .ctx
            .select::<DeployedValidator<{ ProtocolValidator::MintWpAuthPolicy as u8 }>>()
            .hash;
        let weighting_poll_witness = PartialPlutusWitness::new(
            PlutusScriptWitness::Ref(weighting_poll_script_hash),
            weighting_poll::PollAction::Vote.into_pd(),
        );

        let weighting_poll_input = SingleInputBuilder::new(
            TransactionInput::from(weighting_poll.version().output_ref),
            weighting_poll_in.clone(),
        )
        .plutus_script_inline_datum(weighting_poll_witness, RequiredSigners::from(vec![]))
        .unwrap();

        let sorted_inputs = typed_inputs
            .into_iter()
            .map(|(t, _, ex_units)| match t {
                T::Order => (order_input_builder.clone(), ex_units),
                T::VE => (voting_escrow_input.clone(), ex_units),
                T::WPoll => (weighting_poll_input.clone(), ex_units),
            })
            .collect::<Vec<_>>();

        // -----------------------------------------------------------------------------------------
        let mint_weighting_power_policy = weighting_power_deployed_validator.hash;

        let weighting_power_asset_name = compute_epoch_asset_name(weighting_poll.get().epoch);
        let current_posix_time = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as u64;

        let mut wpoll_out = weighting_poll_in.clone();
        let available_weighting_power = voting_escrow.get().voting_power(current_posix_time);

        let distribution = &onchain_order.order.datum.expected_diff;

        let order_weighting_power = distribution.iter().fold(0, |acc, &(_, w)| acc + w);
        println!(
            "available weighting_power: {}, order weighting_power: {}",
            available_weighting_power, order_weighting_power
        );

        if order_weighting_power > available_weighting_power {
            return Err(ExecuteOrderError::WeightingExceedsAvailableVotingPower {
                order_weighting_power,
                available_weighting_power,
            });
        }

        let mut next_weighting_poll = weighting_poll.get().clone();
        next_weighting_poll.apply_votes(distribution);
        next_weighting_poll.weighting_power = Some(order_weighting_power);

        if let Some(data_mut) = wpoll_out.data_mut() {
            unsafe_update_wp_state(data_mut, &next_weighting_poll.distribution);
        }
        wpoll_out.add_asset(
            spectrum_cardano_lib::AssetClass::Token(Token(
                mint_weighting_power_policy,
                AssetName::from(weighting_power_asset_name.clone()),
            )),
            order_weighting_power,
        );

        // Set TX outputs --------------------------------------------------------------------------
        let set_minimal_ada = |output: &mut TransactionOutput, name: &str| {
            let mut amt = output.amount().clone();
            let min_ada = min_ada_required(output, COINS_PER_UTXO_BYTE).unwrap();
            trace!(
                "{} extra lovelaces needed (as computed by CML): {}, orig ada: {}, min_ada: {}",
                name,
                min_ada - amt.coin,
                amt.coin,
                min_ada,
            );
            amt.coin = min_ada;
            output.set_amount(amt);
        };
        set_minimal_ada(&mut wpoll_out, "wpoll_out");
        let weighting_poll_output = SingleOutputBuilderResult::new(wpoll_out.clone());

        // The contract requires voting_escrow_out has index 1. It contains just the minimal amount of ADA.
        set_minimal_ada(&mut voting_escrow_out, "ve_out");
        let voting_escrow_output = SingleOutputBuilderResult::new(voting_escrow_out.clone());

        let outputs = vec![weighting_poll_output, voting_escrow_output];

        // Mint weighting power --------------------------------------------------------------------
        let mint_action = voting_escrow::MintAction::MintPower {
            binder: weighting_poll.get().epoch,
            ve_in_ix: voting_escrow_input_ix,
            proposal_in_ix: wpoll_input_ix,
        };

        let mint_weighting_power_script = PartialPlutusWitness::new(
            PlutusScriptWitness::Ref(mint_weighting_power_policy),
            mint_action.into_pd(),
        );

        let OperatorCreds(operator_pkh, _) = self.ctx.select::<OperatorCreds>();
        let weighting_power_minting_policy = SingleMintBuilder::new_single_asset(
            weighting_power_asset_name.clone(),
            order_weighting_power as i64,
        )
        .plutus_script(
            mint_weighting_power_script,
            RequiredSigners::from(vec![operator_pkh]),
        );
        let token = crate::create_change_output::Token {
            policy_id: mint_weighting_power_policy,
            asset_name: weighting_power_asset_name,
            quantity: order_weighting_power,
        };
        let mint_ex_units = dsd.mint_weighting_power.mint_ex_units.clone();
        let mints = vec![(weighting_power_minting_policy, token, true, mint_ex_units)];

        let OperatorCreds(_operator_pkh, operator_addr) = self.ctx.select::<OperatorCreds>();
        let mut blueprint = DaoTxBlueprint {
            reference_inputs,
            sorted_inputs,
            outputs,
            sorted_mints: mints,
            withdrawal: None,
            fee_buffer: WPOLL_VOTE_ORDER_FEE_DELTA,
            operator_address: operator_addr.clone(),
        };

        let BlueprintEstimates {
            estimated_fee,
            change_output,
            ..
        } = blueprint.compute_estimated_fee_and_change_output();
        assert!(!change_output.output.value().has_multiassets());
        let change_amount = change_output.output.amount().coin;

        // All ADA in the change-output will be placed into `voting_escrow`
        let mut ve_value = blueprint.outputs[1].output.amount().clone();
        ve_value.coin += change_amount;
        blueprint.outputs[1].output.set_amount(ve_value);

        let mut tx_builder = blueprint.build(estimated_fee, None);
        // Set TX validity range
        tx_builder.set_validity_start_interval(current_slot.0);
        tx_builder.set_ttl(current_slot.0 + TX_TTL_SLOT);

        tx_builder
            .add_collateral(InputBuilderResult::from(self.ctx.select::<Collateral>()))
            .unwrap();

        let reward_address = self.ctx.select::<Reward>().0;
        let execution_fee_address: Address = reward_address.into();

        tx_builder.set_fee(estimated_fee);

        // Build tx, change is execution fee.
        let signed_tx_builder = tx_builder
            .build(ChangeSelectionAlgo::Default, &execution_fee_address)
            .unwrap();
        let tx_body = signed_tx_builder.body();

        let tx_hash = hash_transaction_canonical(&tx_body);

        let add_slot = |output_ref| TimedOutputRef {
            output_ref,
            slot: current_slot,
        };

        let next_wp_version = add_slot(OutputRef::new(tx_hash, 0));
        let fresh_wp = Traced::new(
            Predicted(Bundled(
                Snapshot::new(next_weighting_poll, next_wp_version),
                wpoll_out,
            )),
            Some(*prev_wp_version),
        );

        let next_ve_version = OutputRef::new(tx_hash, 1);
        let fresh_ve = Traced::new(
            Predicted(Bundled(
                Snapshot::new(next_ve, next_ve_version),
                voting_escrow_out,
            )),
            Some(*prev_ve_version),
        );

        Ok((signed_tx_builder, fresh_wp, fresh_ve))
    }
}

// The following enums are used to identify input types for particular TXs after lexicographic
// ordering by TxInput.
enum CreateWPollInputType {
    Inflation,
    WPFactory,
    Funding,
}

enum EliminateWPollInputType {
    Funding,
    WPoll,
}

struct MintWPAuthTokensIx {
    factory_in_ix: u64,
    inflation_box_in_ix: u64,
}

fn sort_create_wp_poll_tx_inputs(
    mut inputs: Vec<(CreateWPollInputType, InputBuilderResult)>,
) -> (Vec<InputBuilderResult>, MintWPAuthTokensIx) {
    inputs.sort_by_key(|input| input.1.input.clone());
    let mut inflation_box_in_ix = 0;
    let mut factory_in_ix = 0;
    let input_results: Vec<_> = inputs
        .into_iter()
        .enumerate()
        .map(|(ix, (input_type, input))| {
            match input_type {
                CreateWPollInputType::Inflation => inflation_box_in_ix = ix as u64,
                CreateWPollInputType::WPFactory => factory_in_ix = ix as u64,
                CreateWPollInputType::Funding => (),
            }
            input
        })
        .collect();
    (
        input_results,
        MintWPAuthTokensIx {
            factory_in_ix,
            inflation_box_in_ix,
        },
    )
}
