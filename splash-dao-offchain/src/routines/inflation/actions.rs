use std::ops::{Deref, DerefMut};
use std::time::{SystemTime, UNIX_EPOCH};

use cml_chain::address::{Address, EnterpriseAddress};
use cml_chain::assets::AssetBundle;
use cml_chain::builders::input_builder::{InputBuilderResult, SingleInputBuilder};
use cml_chain::builders::mint_builder::SingleMintBuilder;
use cml_chain::builders::output_builder::{SingleOutputBuilderResult, TransactionOutputBuilder};
use cml_chain::builders::redeemer_builder::RedeemerWitnessKey;
use cml_chain::builders::tx_builder::{ChangeSelectionAlgo, SignedTxBuilder, TransactionUnspentOutput};
use cml_chain::builders::withdrawal_builder::SingleWithdrawalBuilder;
use cml_chain::builders::witness_builder::{PartialPlutusWitness, PlutusScriptWitness};
use cml_chain::certs::{Credential, StakeCredential};
use cml_chain::min_ada::min_ada_required;
use cml_chain::plutus::{ConstrPlutusData, PlutusScript, PlutusV2Script, RedeemerTag};
use cml_chain::transaction::{DatumOption, TransactionInput, TransactionOutput};
use cml_chain::utils::BigInteger;
use cml_chain::{Coin, Deserialize, OrderedHashMap, PolicyId, RequiredSigners};
use cml_core::serialization::FromBytes;
use cml_crypto::{blake2b256, Ed25519Signature, RawBytesEncoding, ScriptHash, TransactionHash};
use log::trace;
use serde::Serialize;
use spectrum_cardano_lib::types::TryFromPData;
use spectrum_cardano_lib::value::ValueExtension;
use spectrum_offchain::domain::event::{Predicted, Traced};
use spectrum_offchain_cardano::deployment::DeployedScriptInfo;

use bloom_offchain::execution_engine::bundled::Bundled;
use spectrum_cardano_lib::collateral::Collateral;
use spectrum_cardano_lib::hash::hash_transaction_canonical;
use spectrum_cardano_lib::plutus_data::{IntoPlutusData, PlutusDataExtension};
use spectrum_cardano_lib::protocol_params::{constant_tx_builder, COINS_PER_UTXO_BYTE};
use spectrum_cardano_lib::transaction::TransactionOutputExtension;
use spectrum_cardano_lib::{AssetClass, AssetName, NetworkId, OutputRef, Token};
use spectrum_offchain::domain::Has;
use spectrum_offchain::ledger::{IntoLedger, TryFromLedger};
use uplc::PlutusData;
use uplc_pallas_primitives::Fragment;

use crate::assets::SPLASH_AC;
use crate::collect_utxos::collect_utxos;
use crate::constants::fee_deltas::{
    CREATE_WPOLL_FEE_DELTA, DISTRIBUTE_INFLATION_FEE_DELTA, ELIMINATE_WPOLL_FEE_DELTA,
    VOTING_ESCROW_VOTING_FEE,
};
use crate::constants::time::{DISTRIBUTE_INFLATION_TX_TTL, MAX_LOCK_TIME_SECONDS, MAX_TIME_DRIFT_MILLIS};
use crate::create_change_output::{ChangeOutputCreator, CreateChangeOutput};
use crate::deployment::{BuiltPolicy, DaoScriptBytes, ProtocolValidator};
use crate::entities::offchain::voting_order::{compute_voting_witness_message, VotingOrder};
use crate::entities::onchain::funding_box::{FundingBox, FundingBoxId, FundingBoxSnapshot};
use crate::entities::onchain::inflation_box::{unsafe_update_ibox_state, INFLATION_BOX_EX_UNITS};
use crate::entities::onchain::make_voting_escrow_order::{
    MakeVotingEscrowOrder, MakeVotingEscrowOrderAction, MakeVotingEscrowOrderBundle,
    MAKE_VOTING_ESCROW_EX_UNITS, MVE_TOKEN_MINT_EX_UNITS,
};
use crate::entities::onchain::permission_manager::{compute_perm_manager_validator, PERM_MANAGER_EX_UNITS};
use crate::entities::onchain::poll_factory::{
    unsafe_update_factory_state, FactoryRedeemer, PollFactoryAction, GOV_PROXY_EX_UNITS, WP_FACTORY_EX_UNITS,
};
use crate::entities::onchain::smart_farm::{
    self, compute_mint_farm_auth_token_validator, FarmId, FARM_EX_UNITS,
};
use crate::entities::onchain::voting_escrow::{
    self, compute_mint_weighting_power_validator, compute_voting_escrow_validator, unsafe_update_ve_state,
    Lock, Owner, VotingEscrow, VotingEscrowAction, VotingEscrowAuthorizedAction, VotingEscrowConfig,
    ORDER_WITNESS_EX_UNITS, VOTING_ESCROW_EX_UNITS, WEIGHTING_POWER_EX_UNITS,
};
use crate::entities::onchain::voting_escrow_factory::{
    exchange_outputs, FactoryAction, VEFactory, VEFactoryDatum, VEFactorySnapshot, VE_FACTORY_EX_UNITS,
};
use crate::entities::onchain::weighting_poll::{
    self, compute_mint_wp_auth_token_validator, unsafe_update_wp_state, MintAction, WeightingPoll,
    MINT_WP_AUTH_EX_UNITS, TOKEN_BURN_EX_UNITS,
};
use crate::entities::Snapshot;
use crate::protocol_config::{
    EDaoMSigAuthPolicy, FarmAuthPolicy, FarmAuthRefScriptOutput, FarmFactoryAuthPolicy, GTAuthPolicy,
    GTBuiltPolicy, GovProxyRefScriptOutput, InflationAuthPolicy, InflationBoxRefScriptOutput,
    MakeVotingEscrowOrderRefScriptOutput, MakeVotingEscrowOrderScriptHash, MintVECompositionPolicy,
    MintVECompositionRefScriptOutput, MintVEIdentifierPolicy, MintVEIdentifierRefScriptOutput,
    MintWPAuthPolicy, MintWPAuthRefScriptOutput, OperatorCreds, PermManagerAuthPolicy,
    PermManagerBoxRefScriptOutput, PollFactoryRefScriptOutput, Reward, SplashPolicy, VEFactoryAuthPolicy,
    VEFactoryRefScriptOutput, VEFactoryScriptHash, VotingEscrowRefScriptOutput, VotingEscrowScriptHash,
    WPFactoryAuthPolicy, WeightingPowerPolicy, WeightingPowerRefScriptOutput, TX_FEE_CORRECTION,
};
use crate::routines::inflation::TimedOutputRef;
use crate::time::NetworkTimeProvider;
use crate::util::set_min_ada;
use crate::{GenesisEpochStartTime, NetworkTimeSource};

use super::{
    AvailableFundingBoxes, FundingBoxChanges, InflationBoxSnapshot, PermManagerSnapshot, PollFactorySnapshot,
    Slot, SmartFarmSnapshot, VotingEscrowSnapshot, WeightingPollSnapshot,
};

#[async_trait::async_trait]
pub trait InflationActions<Bearer> {
    async fn create_wpoll(
        &self,
        inflation_box: Bundled<InflationBoxSnapshot, Bearer>,
        factory: Bundled<PollFactorySnapshot, Bearer>,
        current_slot: Slot,
        funding_boxes: AvailableFundingBoxes,
    ) -> (
        SignedTxBuilder,
        Traced<Predicted<Bundled<InflationBoxSnapshot, Bearer>>>,
        Traced<Predicted<Bundled<PollFactorySnapshot, Bearer>>>,
        Traced<Predicted<Bundled<WeightingPollSnapshot, Bearer>>>,
        FundingBoxChanges,
    );
    async fn eliminate_wpoll(
        &self,
        weighting_poll: Bundled<WeightingPollSnapshot, Bearer>,
        funding_boxes: AvailableFundingBoxes,
        current_slot: Slot,
    ) -> (SignedTxBuilder, FundingBoxChanges);
    async fn execute_order(
        &self,
        weighting_poll: Bundled<WeightingPollSnapshot, Bearer>,
        order: (VotingOrder, Bundled<VotingEscrowSnapshot, Bearer>),
        current_slot: Slot,
    ) -> Result<
        (
            SignedTxBuilder,
            Traced<Predicted<Bundled<WeightingPollSnapshot, Bearer>>>,
            Traced<Predicted<Bundled<VotingEscrowSnapshot, Bearer>>>,
        ),
        ExecuteOrderError,
    >;
    async fn distribute_inflation(
        &self,
        weighting_poll: Bundled<WeightingPollSnapshot, Bearer>,
        farm: Bundled<SmartFarmSnapshot, Bearer>,
        perm_manager: Bundled<PermManagerSnapshot, Bearer>,
        current_slot: Slot,
        farm_weight: u64,
        funding_boxes: AvailableFundingBoxes,
    ) -> (
        SignedTxBuilder,
        Traced<Predicted<Bundled<WeightingPollSnapshot, Bearer>>>,
        Traced<Predicted<Bundled<SmartFarmSnapshot, Bearer>>>,
        FundingBoxChanges,
    );
    async fn make_voting_escrow(
        &self,
        make_voting_escrow_order: MakeVotingEscrowOrderBundle<Bearer>,
        ve_factory: Bundled<VEFactorySnapshot, Bearer>,
        current_slot: Slot,
    ) -> Result<
        (
            SignedTxBuilder,
            Traced<Predicted<Bundled<VEFactorySnapshot, Bearer>>>,
            Traced<Predicted<Bundled<VotingEscrowSnapshot, Bearer>>>,
        ),
        MakeVotingEscrowError,
    >;
}

/// 1/5 of MAX_TIME_DRIFT
const TX_TTL_SLOT: u64 = MAX_TIME_DRIFT_MILLIS / 1000 / 5;

#[derive(derive_more::From)]
pub struct CardanoInflationActions<Ctx> {
    ctx: Ctx,
}

#[async_trait::async_trait]
impl<Ctx> InflationActions<TransactionOutput> for CardanoInflationActions<Ctx>
where
    Ctx: Send
        + Sync
        + Clone
        + Has<Reward>
        + Has<Collateral>
        + Has<SplashPolicy>
        + Has<InflationBoxRefScriptOutput>
        + Has<InflationAuthPolicy>
        + Has<DeployedScriptInfo<{ ProtocolValidator::Inflation as u8 }>>
        + Has<PollFactoryRefScriptOutput>
        + Has<DeployedScriptInfo<{ ProtocolValidator::WpFactory as u8 }>>
        + Has<MintWPAuthPolicy>
        + Has<MintWPAuthRefScriptOutput>
        + Has<MintVEIdentifierPolicy>
        + Has<MintVEIdentifierRefScriptOutput>
        + Has<FarmAuthPolicy>
        + Has<FarmAuthRefScriptOutput>
        + Has<FarmFactoryAuthPolicy>
        + Has<VEFactoryScriptHash>
        + Has<VEFactoryRefScriptOutput>
        + Has<MintVECompositionPolicy>
        + Has<MintVECompositionRefScriptOutput>
        + Has<MakeVotingEscrowOrderScriptHash>
        + Has<MakeVotingEscrowOrderRefScriptOutput>
        + Has<VotingEscrowScriptHash>
        + Has<VotingEscrowRefScriptOutput>
        + Has<WeightingPowerPolicy>
        + Has<WeightingPowerRefScriptOutput>
        + Has<PermManagerBoxRefScriptOutput>
        + Has<GovProxyRefScriptOutput>
        + Has<EDaoMSigAuthPolicy>
        + Has<PermManagerAuthPolicy>
        + Has<GTAuthPolicy>
        + Has<GTBuiltPolicy>
        + Has<NetworkId>
        + Has<OperatorCreds>
        + Has<GenesisEpochStartTime>
        + Has<DeployedScriptInfo<{ ProtocolValidator::GovProxy as u8 }>>,
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

        let inflation_script_hash = self
            .ctx
            .select::<DeployedScriptInfo<{ ProtocolValidator::Inflation as u8 }>>()
            .script_hash;
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

        tx_builder.add_reference_input(self.ctx.select::<InflationBoxRefScriptOutput>().0.clone());

        let prev_ib_version = *inflation_box.version();
        let (next_inflation_box, emission_rate) = inflation_box.get().release_next_tranche();
        let mut inflation_box_out = inflation_box_in.clone();
        if let Some(data_mut) = inflation_box_out.data_mut() {
            // Following unwrap is safe due to the `.release_next_trache()` call above.
            unsafe_update_ibox_state(data_mut, next_inflation_box.last_processed_epoch.unwrap() + 1);
        }
        inflation_box_out.sub_asset(*SPLASH_AC, emission_rate.untag());
        set_min_ada(&mut inflation_box_out);
        let inflation_output = SingleOutputBuilderResult::new(inflation_box_out.clone());

        // WP factory

        let wp_factory_script_hash = self
            .ctx
            .select::<DeployedScriptInfo<{ ProtocolValidator::WpFactory as u8 }>>()
            .script_hash;

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

        tx_builder.add_reference_input(self.ctx.select::<PollFactoryRefScriptOutput>().0.clone());

        let prev_factory_version = *factory.version();
        let (next_factory, fresh_wpoll) = factory.unwrap().next_weighting_poll(emission_rate);
        let mut factory_out = factory_in;
        if let Some(data_mut) = factory_out.data_mut() {
            unsafe_update_factory_state(data_mut, next_factory.last_poll_epoch.unwrap());
        }

        let (input_results, funding_boxes_to_spend) =
            select_funding_boxes(10_000_000, vec![], funding_boxes.0, &self.ctx);

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

        for input in input_results {
            change_output_creator.add_input(&input);
            tx_builder.add_input(input).unwrap();
        }

        if inflation_box_in_ix < factory_in_ix {
            tx_builder.set_exunits(
                RedeemerWitnessKey::new(RedeemerTag::Spend, inflation_box_in_ix),
                INFLATION_BOX_EX_UNITS,
            );
            tx_builder.set_exunits(
                RedeemerWitnessKey::new(RedeemerTag::Spend, factory_in_ix),
                WP_FACTORY_EX_UNITS,
            );
        } else {
            tx_builder.set_exunits(
                RedeemerWitnessKey::new(RedeemerTag::Spend, factory_in_ix),
                WP_FACTORY_EX_UNITS,
            );
            tx_builder.set_exunits(
                RedeemerWitnessKey::new(RedeemerTag::Spend, inflation_box_in_ix),
                INFLATION_BOX_EX_UNITS,
            );
        }

        let mint_action = MintAction::MintAuthToken {
            factory_in_ix: factory_in_ix as u32,
            inflation_box_in_ix: inflation_box_in_ix as u32,
        };

        let wp_auth_policy = self.ctx.select::<MintWPAuthPolicy>().0;
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
        tx_builder.add_reference_input(self.ctx.select::<MintWPAuthRefScriptOutput>().0.clone());
        tx_builder.add_mint(wp_auth_minting_policy).unwrap();
        tx_builder.set_exunits(
            RedeemerWitnessKey::new(RedeemerTag::Mint, 0),
            MINT_WP_AUTH_EX_UNITS,
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

        let spent_funding_boxes: Vec<_> = funding_boxes_to_spend.into_iter().map(|f| f.id).collect();

        let funding_box_changes = FundingBoxChanges {
            spent: spent_funding_boxes,
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
        funding_boxes: AvailableFundingBoxes,
        current_slot: Slot,
    ) -> (SignedTxBuilder, FundingBoxChanges) {
        let mut tx_builder = constant_tx_builder();
        tx_builder.set_validity_start_interval(current_slot.0);
        tx_builder.set_ttl(current_slot.0 + TX_TTL_SLOT);

        let mint_weighting_power_ref_script = self.ctx.select::<WeightingPowerRefScriptOutput>().0;
        let wpoll_auth_ref_script = self.ctx.select::<MintWPAuthRefScriptOutput>().0;
        let wpoll_script_hash = self.ctx.select::<MintWPAuthPolicy>().0;

        let redeemer = weighting_poll::PollAction::Destroy;
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
            select_funding_boxes(3_000_000, vec![], funding_boxes.0, &self.ctx);

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

        tx_builder.add_reference_input(mint_weighting_power_ref_script);
        tx_builder.add_reference_input(wpoll_auth_ref_script);
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
        let mint_weighting_power_policy = self.ctx.select::<WeightingPowerPolicy>().0;
        let weighting_power = weighting_poll.get().weighting_power.unwrap();
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

        tx_builder.set_exunits(RedeemerWitnessKey::new(RedeemerTag::Mint, 0), TOKEN_BURN_EX_UNITS);
        tx_builder.set_exunits(RedeemerWitnessKey::new(RedeemerTag::Mint, 1), TOKEN_BURN_EX_UNITS);

        // ------------------
        change_output_creator.burn_token(crate::create_change_output::Token {
            policy_id: mint_weighting_power_policy,
            asset_name: mint_weighting_power_token_name,
            quantity: weighting_power,
        });

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
            MINT_WP_AUTH_EX_UNITS,
        );

        let estimated_tx_fee = tx_builder.min_fee(true).unwrap() + ELIMINATE_WPOLL_FEE_DELTA;
        let change_output =
            change_output_creator.create_change_output(estimated_tx_fee, operator_addr.clone());
        tx_builder.add_output(change_output).unwrap();
        tx_builder.set_fee(estimated_tx_fee);

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

        let spent_funding_boxes: Vec<_> = funding_boxes_to_spend.into_iter().map(|f| f.id).collect();

        let funding_box_changes = FundingBoxChanges {
            spent: spent_funding_boxes,
            created: created_funding_boxes,
        };

        (signed_tx_builder, funding_box_changes)
    }

    async fn execute_order(
        &self,
        Bundled(weighting_poll, weighting_poll_in): Bundled<WeightingPollSnapshot, TransactionOutput>,
        (mut order, Bundled(voting_escrow, ve_box_in)): (
            VotingOrder,
            Bundled<VotingEscrowSnapshot, TransactionOutput>,
        ),
        current_slot: Slot,
    ) -> Result<
        (
            SignedTxBuilder,
            Traced<Predicted<Bundled<WeightingPollSnapshot, TransactionOutput>>>,
            Traced<Predicted<Bundled<VotingEscrowSnapshot, TransactionOutput>>>,
        ),
        ExecuteOrderError,
    > {
        let mut tx_builder = constant_tx_builder();

        let prev_ve_version = voting_escrow.version();
        let prev_wp_version = weighting_poll.version();

        // Voting escrow ---------------------------------------------------------------------------
        let mut voting_escrow_out = ve_box_in.clone();
        let data_mut = voting_escrow_out.data_mut().unwrap();
        let VotingEscrowConfig {
            owner,
            last_wp_epoch,
            version,
            ..
        } = VotingEscrowConfig::try_from_pd(data_mut.clone()).unwrap();

        // Verify that witness is authorized by the owner.
        if let Owner::PubKey(bytes) = owner {
            let pk = cml_crypto::PublicKey::from_raw_bytes(&bytes)
                .map_err(|_| ExecuteOrderError::Other("Can't extrat PublicKey from bytes".into()))?;
            let signature = Ed25519Signature::from_raw_bytes(&order.proof)
                .map_err(|_| ExecuteOrderError::Other("Can't extract Ed25519Signature from bytes".into()))?;
            println!("witness_script hash: {}", order.witness.to_hex());
            println!("redeemer: {}", order.witness_input);
            println!("version: {}", order.version);
            let message = compute_voting_witness_message(
                order.witness,
                order.witness_input.clone(),
                order.version as u64,
            )
            .map_err(|_| ExecuteOrderError::WeightingWitness(WeightingWitnessError::CannotDecodeRedeemer))?;
            println!("message: {}", hex::encode(&message));
            if !pk.verify(&message, &signature) {
                return Err(ExecuteOrderError::WeightingWitness(
                    WeightingWitnessError::OwnerAuthFailure,
                ));
            }
        }

        let new_wp_epoch = weighting_poll.get().epoch;

        // Check `ve_is_eligible_to_vote_in_this_epoch` predicate from `mint_weighting_power`.
        if last_wp_epoch >= new_wp_epoch as i32 {
            return Err(ExecuteOrderError::WeightingWitness(
                WeightingWitnessError::VotingEscrowIneligibleToVote {
                    last_wp_epoch,
                    current_epoch: new_wp_epoch as i32,
                },
            ));
        }

        if version != order.version {
            return Err(ExecuteOrderError::WeightingWitness(
                WeightingWitnessError::VotingEscrowVersionMismatch {
                    voting_escrow_version: version,
                    order_version: order.version,
                },
            ));
        }

        let new_ve_version = voting_escrow.get().version + 1;
        unsafe_update_ve_state(data_mut, new_wp_epoch, new_ve_version);
        let mut next_ve = voting_escrow.get().clone();
        next_ve.last_wp_epoch = new_wp_epoch as i32;
        next_ve.version = new_ve_version;

        let mut ve_amt = voting_escrow_out.amount().clone();
        ve_amt.sub_unsafe(AssetClass::Native, VOTING_ESCROW_VOTING_FEE);
        dbg!(&ve_amt);
        voting_escrow_out.set_amount(ve_amt);

        let authorized_action = VotingEscrowAuthorizedAction {
            action: VotingEscrowAction::Governance,
            witness: order.witness,
            version: order.version,
            signature: order.proof,
        };

        let voting_escrow_ref_script = self.ctx.select::<VotingEscrowRefScriptOutput>().0;
        let wpoll_auth_ref_script = self.ctx.select::<MintWPAuthRefScriptOutput>().0;
        let weighting_power_ref_script = self.ctx.select::<WeightingPowerRefScriptOutput>().0;
        let voting_escrow_script_hash = self.ctx.select::<VotingEscrowScriptHash>().0;

        let voting_escrow_witness = PartialPlutusWitness::new(
            PlutusScriptWitness::Ref(voting_escrow_script_hash),
            authorized_action.into_pd(),
        );

        let voting_escrow_input = SingleInputBuilder::new(
            TransactionInput::from(voting_escrow.version().output_ref),
            ve_box_in.clone(),
        )
        .plutus_script_inline_datum(voting_escrow_witness, RequiredSigners::from(vec![]))
        .unwrap();

        let voting_escrow_input_tx_hash = voting_escrow_input.input.transaction_id;

        let mut change_output_creator = ChangeOutputCreator::default();

        tx_builder.add_reference_input(voting_escrow_ref_script);
        change_output_creator.add_input(&voting_escrow_input);
        tx_builder.add_input(voting_escrow_input).unwrap();

        // weighting_poll --------------------------------------------------------------------------
        let weighting_poll_script_hash = self.ctx.select::<MintWPAuthPolicy>().0;
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

        let weighting_poll_input_tx_hash = weighting_poll_input.input.transaction_id;

        tx_builder.add_reference_input(wpoll_auth_ref_script);
        change_output_creator.add_input(&weighting_poll_input);
        tx_builder.add_input(weighting_poll_input).unwrap();

        let mint_weighting_power_policy = self.ctx.select::<WeightingPowerPolicy>().0;

        let weighting_power_asset_name = compute_epoch_asset_name(weighting_poll.get().epoch);
        let current_posix_time = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as u64;

        let mut wpoll_out = weighting_poll_in.clone();
        let weighting_power = voting_escrow.get().voting_power(current_posix_time);
        println!("weighting_power: {}", weighting_power);

        let distribution_weighting_power = order.distribution.iter().fold(0, |acc, &(_, w)| acc + w);
        if distribution_weighting_power != weighting_power {
            return Err(ExecuteOrderError::WeightingExceedsAvailableVotingPower);
        }

        let mut next_weighting_poll = weighting_poll.get().clone();
        next_weighting_poll.apply_votes(&order.distribution);
        next_weighting_poll.weighting_power = Some(weighting_power);

        if let Some(data_mut) = wpoll_out.data_mut() {
            unsafe_update_wp_state(data_mut, &next_weighting_poll.distribution);
        }
        wpoll_out.add_asset(
            spectrum_cardano_lib::AssetClass::Token(Token(
                mint_weighting_power_policy,
                AssetName::from(weighting_power_asset_name.clone()),
            )),
            weighting_power,
        );

        // Set TX outputs --------------------------------------------------------------------------
        let mut amt = wpoll_out.amount().clone();
        let min_ada = min_ada_required(&wpoll_out, COINS_PER_UTXO_BYTE).unwrap();
        trace!(
            "wpoll_out extra lovelaces needed (as computed by CML): {}, orig ada: {}, min_ada: {}",
            min_ada - amt.coin,
            amt.coin,
            min_ada,
        );
        amt.coin = min_ada;
        wpoll_out.set_amount(amt);
        let weighting_poll_output = SingleOutputBuilderResult::new(wpoll_out.clone());
        change_output_creator.add_output(&weighting_poll_output);
        tx_builder.add_output(weighting_poll_output).unwrap();

        // The contract requires voting_escrow_out has index 1
        let voting_escrow_output = SingleOutputBuilderResult::new(voting_escrow_out.clone());
        change_output_creator.add_output(&voting_escrow_output);
        tx_builder.add_output(voting_escrow_output).unwrap();

        // Mint weighting power --------------------------------------------------------------------
        let mint_action = if voting_escrow_input_tx_hash < weighting_poll_input_tx_hash {
            tx_builder.set_exunits(
                RedeemerWitnessKey::new(RedeemerTag::Spend, 0),
                VOTING_ESCROW_EX_UNITS,
            );
            tx_builder.set_exunits(
                RedeemerWitnessKey::new(RedeemerTag::Spend, 1),
                MINT_WP_AUTH_EX_UNITS,
            );
            voting_escrow::MintAction::MintPower {
                binder: weighting_poll.get().epoch,
                ve_in_ix: 0,
                proposal_in_ix: 1,
            }
        } else {
            tx_builder.set_exunits(
                RedeemerWitnessKey::new(RedeemerTag::Spend, 1),
                VOTING_ESCROW_EX_UNITS,
            );
            tx_builder.set_exunits(
                RedeemerWitnessKey::new(RedeemerTag::Spend, 0),
                MINT_WP_AUTH_EX_UNITS,
            );
            voting_escrow::MintAction::MintPower {
                binder: weighting_poll.get().epoch,
                ve_in_ix: 1,
                proposal_in_ix: 0,
            }
        };

        let mint_weighting_power_script = PartialPlutusWitness::new(
            PlutusScriptWitness::Ref(mint_weighting_power_policy),
            mint_action.into_pd(),
        );

        let OperatorCreds(operator_pkh, _) = self.ctx.select::<OperatorCreds>();
        let weighting_power_minting_policy =
            SingleMintBuilder::new_single_asset(weighting_power_asset_name, weighting_power as i64)
                .plutus_script(
                    mint_weighting_power_script,
                    RequiredSigners::from(vec![operator_pkh]),
                );
        tx_builder.add_reference_input(weighting_power_ref_script);
        tx_builder.add_mint(weighting_power_minting_policy).unwrap();
        tx_builder.set_exunits(
            RedeemerWitnessKey::new(RedeemerTag::Mint, 0),
            WEIGHTING_POWER_EX_UNITS,
        );

        // Set witness script (needed by voting_escrow) --------------------------------------------
        let withdrawal_address = cml_chain::address::RewardAddress::new(
            self.ctx.select::<NetworkId>().into(),
            Credential::new_script(order.witness),
        );

        // TODO: ACTUAL SCRIPT HERE
        let voting_witness_script = PlutusScript::PlutusV2(PlutusV2Script::new(
            hex::decode(&DaoScriptBytes::global().voting_witness).unwrap(),
        ));

        let witness_input =
            cml_chain::plutus::PlutusData::from_cbor_bytes(&hex::decode(order.witness_input).unwrap())
                .map_err(|_| {
                    ExecuteOrderError::WeightingWitness(WeightingWitnessError::CannotDecodeRedeemer)
                })?;
        let order_witness =
            PartialPlutusWitness::new(PlutusScriptWitness::Script(voting_witness_script), witness_input);
        let withdrawal_result = SingleWithdrawalBuilder::new(withdrawal_address, 0)
            .plutus_script(order_witness, RequiredSigners::from(vec![]))
            .unwrap();
        tx_builder.add_withdrawal(withdrawal_result);
        tx_builder.set_exunits(
            RedeemerWitnessKey::new(RedeemerTag::Reward, 0),
            ORDER_WITNESS_EX_UNITS,
        );

        // Set TX validity range
        tx_builder.set_validity_start_interval(current_slot.0);
        tx_builder.set_ttl(current_slot.0 + TX_TTL_SLOT);

        tx_builder
            .add_collateral(InputBuilderResult::from(self.ctx.select::<Collateral>()))
            .unwrap();

        let reward_address = self.ctx.select::<Reward>().0;
        let execution_fee_address: Address = reward_address.into();

        tx_builder.set_fee(change_output_creator.input_coin - change_output_creator.output_coin);

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

        let next_ve_version = add_slot(OutputRef::new(tx_hash, 1));
        let fresh_ve = Traced::new(
            Predicted(Bundled(
                Snapshot::new(next_ve, next_ve_version),
                voting_escrow_out,
            )),
            Some(*prev_ve_version),
        );

        Ok((signed_tx_builder, fresh_wp, fresh_ve))
    }

    async fn distribute_inflation(
        &self,
        Bundled(weighting_poll, weighting_poll_in): Bundled<WeightingPollSnapshot, TransactionOutput>,
        Bundled(farm, farm_in): Bundled<SmartFarmSnapshot, TransactionOutput>,
        Bundled(perm_manager, perm_manager_in): Bundled<PermManagerSnapshot, TransactionOutput>,
        current_slot: Slot,
        farm_weight: u64,
        funding_boxes: AvailableFundingBoxes,
    ) -> (
        SignedTxBuilder,
        Traced<Predicted<Bundled<WeightingPollSnapshot, TransactionOutput>>>,
        Traced<Predicted<Bundled<SmartFarmSnapshot, TransactionOutput>>>,
        FundingBoxChanges,
    ) {
        let mut tx_builder = constant_tx_builder();

        let wpoll_auth_ref_script = self.ctx.select::<MintWPAuthRefScriptOutput>().0;
        let smart_farm_ref_script = self.ctx.select::<FarmAuthRefScriptOutput>().0;

        let mut next_weighting_poll = weighting_poll.get().clone();
        let farm_distribution_ix = next_weighting_poll
            .distribution
            .iter()
            .position(|&(farm_id, _)| farm_id == farm.get().farm_id)
            .unwrap();
        let old_weight = next_weighting_poll.distribution[farm_distribution_ix].1;
        assert!(old_weight >= farm_weight);
        next_weighting_poll.distribution[farm_distribution_ix].1 = old_weight - farm_weight;

        let weighting_poll_script_hash = self.ctx.select::<MintWPAuthPolicy>().0;

        let (input_results, funding_boxes_to_spend) =
            select_funding_boxes(10_000_000, vec![], funding_boxes.0, &self.ctx);

        let mut typed_inputs: Vec<_> = input_results
            .into_iter()
            .map(|input| (input.input.clone(), DistributeInflationInputType::Funding(input)))
            .collect();

        typed_inputs.extend([
            (
                TransactionInput::from(weighting_poll.version().output_ref),
                DistributeInflationInputType::WPoll,
            ),
            (
                TransactionInput::from(farm.version().output_ref),
                DistributeInflationInputType::Farm,
            ),
        ]);

        typed_inputs.sort_by_key(|(tx_hash, _)| tx_hash.clone());
        let farm_in_ix = typed_inputs
            .iter()
            .position(|(_, t)| matches!(t, DistributeInflationInputType::Farm))
            .unwrap() as u32;

        let OperatorCreds(operator_pkh, operator_addr) = self.ctx.select::<OperatorCreds>();

        let perm_manager_unspent_input = TransactionUnspentOutput::new(
            TransactionInput::from(perm_manager.version().output_ref),
            perm_manager_in.clone(),
        );
        tx_builder.add_reference_input(perm_manager_unspent_input.clone());

        let mut change_output_creator = ChangeOutputCreator::default();
        for (i, (_, input_type)) in typed_inputs.into_iter().enumerate() {
            match input_type {
                DistributeInflationInputType::WPoll => {
                    let redeemer = weighting_poll::PollAction::Distribute {
                        farm_ix: farm_distribution_ix as u32,
                        farm_in_ix,
                    };
                    let weighting_poll_script = PartialPlutusWitness::new(
                        PlutusScriptWitness::Ref(weighting_poll_script_hash),
                        redeemer.into_pd(),
                    );

                    let weighting_poll_input = SingleInputBuilder::new(
                        TransactionInput::from(weighting_poll.version().output_ref),
                        weighting_poll_in.clone(),
                    )
                    .plutus_script_inline_datum(weighting_poll_script, RequiredSigners::from(vec![]))
                    .unwrap();
                    tx_builder.add_reference_input(wpoll_auth_ref_script.clone());
                    change_output_creator.add_input(&weighting_poll_input);
                    tx_builder.add_input(weighting_poll_input).unwrap();
                    tx_builder.set_exunits(
                        RedeemerWitnessKey::new(RedeemerTag::Spend, i as u64),
                        MINT_WP_AUTH_EX_UNITS,
                    );
                }

                DistributeInflationInputType::Farm => {
                    // First determine the index of `perm_manager` within `reference_input`
                    let mut indexed_inputs = vec![
                        (
                            smart_farm_ref_script.input.clone(),
                            DistributeInflationRefInputType::Other,
                        ),
                        (
                            wpoll_auth_ref_script.input.clone(),
                            DistributeInflationRefInputType::Other,
                        ),
                        (
                            perm_manager_unspent_input.input.clone(),
                            DistributeInflationRefInputType::PermManager,
                        ),
                    ];
                    indexed_inputs.sort_by_key(|(input, _)| input.clone());
                    let perm_manager_input_ix = indexed_inputs
                        .iter()
                        .position(|(_, typ)| matches!(typ, DistributeInflationRefInputType::PermManager))
                        .unwrap() as u32;

                    let redeemer = smart_farm::Redeemer {
                        successor_out_ix: 1,
                        action: smart_farm::Action::DistributeRewards {
                            perm_manager_input_ix,
                        },
                    }
                    .into_pd();

                    let smart_farm_script_hash = self.ctx.select::<FarmAuthPolicy>().0;
                    let smart_farm_script =
                        PartialPlutusWitness::new(PlutusScriptWitness::Ref(smart_farm_script_hash), redeemer);

                    let smart_farm_input = SingleInputBuilder::new(
                        TransactionInput::from(farm.version().output_ref),
                        farm_in.clone(),
                    )
                    .plutus_script_inline_datum(smart_farm_script, RequiredSigners::from(vec![operator_pkh]))
                    .unwrap();
                    tx_builder.add_reference_input(smart_farm_ref_script.clone());
                    change_output_creator.add_input(&smart_farm_input);
                    tx_builder.add_input(smart_farm_input).unwrap();
                    tx_builder.set_exunits(
                        RedeemerWitnessKey::new(RedeemerTag::Spend, i as u64),
                        FARM_EX_UNITS,
                    );
                }

                DistributeInflationInputType::Funding(funding_input) => {
                    change_output_creator.add_input(&funding_input);
                    tx_builder.add_input(funding_input).unwrap();
                }
            }
        }

        dbg!(weighting_poll.get());

        // Adjust splash values in weighting_poll and farm.
        let splash_emission = weighting_poll.get().emission_rate.untag() * farm_weight
            / weighting_poll.get().weighting_power.unwrap();

        let mut weighting_poll_out = weighting_poll_in.clone();
        weighting_poll_out.sub_asset(*SPLASH_AC, splash_emission);

        let mut farm_out = farm_in.clone();
        farm_out.add_asset(*SPLASH_AC, splash_emission);

        // Reduce weightings in weighting_poll's datum
        if let Some(data_mut) = weighting_poll_out.data_mut() {
            unsafe_update_wp_state(data_mut, &next_weighting_poll.distribution);
        }

        // farm output must be at index 1
        let weighting_poll_output = SingleOutputBuilderResult::new(weighting_poll_out.clone());
        let farm_output = SingleOutputBuilderResult::new(farm_out.clone());
        change_output_creator.add_output(&weighting_poll_output);
        tx_builder.add_output(weighting_poll_output).unwrap();
        change_output_creator.add_output(&farm_output);
        tx_builder.add_output(farm_output).unwrap();

        // Add operator as signatory
        tx_builder.add_required_signer(operator_pkh);

        tx_builder
            .add_collateral(InputBuilderResult::from(self.ctx.select::<Collateral>()))
            .unwrap();

        let estimated_tx_fee = tx_builder.min_fee(true).unwrap() + DISTRIBUTE_INFLATION_FEE_DELTA;
        let change_output =
            change_output_creator.create_change_output(estimated_tx_fee, operator_addr.clone());
        tx_builder.add_output(change_output).unwrap();
        tx_builder.set_fee(estimated_tx_fee);
        tx_builder.set_validity_start_interval(current_slot.0);
        tx_builder.set_ttl(current_slot.0 + DISTRIBUTE_INFLATION_TX_TTL);

        // Build tx, change is execution fee.
        let signed_tx_builder = tx_builder
            .build(ChangeSelectionAlgo::Default, &operator_addr)
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

        let spent_funding_boxes: Vec<_> = funding_boxes_to_spend.into_iter().map(|f| f.id).collect();

        let funding_box_changes = FundingBoxChanges {
            spent: spent_funding_boxes,
            created: created_funding_boxes,
        };
        let next_wp_version = add_slot(OutputRef::new(tx_hash, 0));
        let fresh_wp = Traced::new(
            Predicted(Bundled(
                Snapshot::new(next_weighting_poll, next_wp_version),
                weighting_poll_out,
            )),
            Some(*weighting_poll.version()),
        );

        let next_farm_version = add_slot(OutputRef::new(tx_hash, 1));
        let next_farm = farm.get().clone();
        let fresh_farm = Traced::new(
            Predicted(Bundled(Snapshot::new(next_farm, next_farm_version), farm_out)),
            Some(*farm.version()),
        );

        (signed_tx_builder, fresh_wp, fresh_farm, funding_box_changes)
    }

    async fn make_voting_escrow(
        &self,
        MakeVotingEscrowOrderBundle {
            order,
            output_ref: mve_output_ref,
            bearer: mve_tx_output,
            ..
        }: MakeVotingEscrowOrderBundle<TransactionOutput>,
        Bundled(ve_factory, ve_factory_in): Bundled<VEFactorySnapshot, TransactionOutput>,
        current_slot: Slot,
    ) -> Result<
        (
            SignedTxBuilder,
            Traced<Predicted<Bundled<VEFactorySnapshot, TransactionOutput>>>,
            Traced<Predicted<Bundled<VotingEscrowSnapshot, TransactionOutput>>>,
        ),
        MakeVotingEscrowError,
    > {
        let time_source = NetworkTimeSource;
        let locktime_exceeds_limit = match order.ve_datum.locked_until {
            Lock::Def(until) => {
                let now_in_seconds = time_source.network_time().await;
                (until / 1000) - now_in_seconds > MAX_LOCK_TIME_SECONDS
            }
            Lock::Indef(duration) => duration.as_secs() > MAX_LOCK_TIME_SECONDS,
        };
        if locktime_exceeds_limit {
            return Err(MakeVotingEscrowError::LocktimeExceedsLimit);
        }

        let ve_factory_in_value = ve_factory_in.value();
        let mut ve_factory_out_value = ve_factory_in_value.clone();
        let mve_coin = mve_tx_output.value().coin;

        // Deposit assets into ve_factory -------------------------------------------
        let accepted_assets = ve_factory.get().accepted_assets.clone();

        for (script_hash, names) in mve_tx_output.value().multiasset.iter() {
            for (name, qty) in names.iter() {
                let token = Token(*script_hash, AssetName::from(name.clone()));
                let accepted_asset = accepted_assets.iter().any(|(tok, _)| *tok == token);
                let ac = AssetClass::from(token);
                if accepted_asset {
                    ve_factory_out_value.add_unsafe(ac, *qty);
                } else {
                    return Err(MakeVotingEscrowError::NonAcceptedAsset);
                }
            }
        }

        let ve_composition_policy = self.ctx.select::<MintVECompositionPolicy>().0;

        let (ve_composition_qty, mut voting_escrow_value) = exchange_outputs(
            ve_factory_in_value,
            &ve_factory_out_value,
            accepted_assets.clone(),
            ve_composition_policy,
            false,
        );
        let mut next_ve_factory = ve_factory.get().clone();
        next_ve_factory.gt_tokens_available -= ve_composition_qty;

        // `ve_factory` will loan `ve_composition_qty` GT tokens to the newly created `voting_escrow`.
        let gt_token = self.ctx.select::<GTBuiltPolicy>().0;
        let gt_auth_name = spectrum_cardano_lib::AssetName::from(gt_token.asset_name.clone());
        let gt_ac = AssetClass::from(Token(gt_token.policy_id, gt_auth_name));
        ve_factory_out_value.sub_unsafe(gt_ac, ve_composition_qty);

        let ve_factory_output_ref = ve_factory.version().output_ref;
        let (ve_factory_in_ix, mve_in_ix) = if ve_factory_output_ref < mve_output_ref.output_ref {
            (0, 1)
        } else {
            (1, 0)
        };

        let ve_factory_unspent_output = self.ctx.select::<VEFactoryRefScriptOutput>().0;

        let mve_unspent_output = self.ctx.select::<MakeVotingEscrowOrderRefScriptOutput>().0;

        let mut change_output_creator = ChangeOutputCreator::default();
        let mut tx_builder = constant_tx_builder();
        tx_builder.add_reference_input(ve_factory_unspent_output);
        tx_builder.add_reference_input(self.ctx.select::<VotingEscrowRefScriptOutput>().0);
        tx_builder.add_reference_input(self.ctx.select::<MintVECompositionRefScriptOutput>().0);
        tx_builder.add_reference_input(self.ctx.select::<MintVEIdentifierRefScriptOutput>().0);
        tx_builder.add_reference_input(mve_unspent_output);

        // `ve_factory` input ----------------------------------------------------------------------
        let ve_factory_datum = if let Some(datum) = ve_factory_in.datum() {
            datum
        } else {
            return Err(MakeVotingEscrowError::VEFactoryDatumNotPresent);
        };

        let ve_factory_script_hash = self.ctx.select::<VEFactoryScriptHash>().0;
        let ve_factory_redeemer = FactoryAction::Deposit.into_pd();
        let ve_factory_witness = PartialPlutusWitness::new(
            PlutusScriptWitness::Ref(ve_factory_script_hash),
            ve_factory_redeemer,
        );

        let ve_factory_input_builder =
            SingleInputBuilder::new(TransactionInput::from(ve_factory_output_ref), ve_factory_in)
                .plutus_script_inline_datum(ve_factory_witness, vec![].into())
                .unwrap();
        change_output_creator.add_input(&ve_factory_input_builder);
        tx_builder.add_input(ve_factory_input_builder.clone()).unwrap();

        // `make_voting_escrow_order` input --------------------------------------------------------
        let mve_script_hash = self.ctx.select::<MakeVotingEscrowOrderScriptHash>().0;
        let mve_redeemer = MakeVotingEscrowOrderAction::Deposit {
            ve_factory_input_ix: ve_factory_in_ix,
        }
        .into_pd();
        let mve_witness = PartialPlutusWitness::new(PlutusScriptWitness::Ref(mve_script_hash), mve_redeemer);

        let mve_input_builder =
            SingleInputBuilder::new(TransactionInput::from(mve_output_ref.output_ref), mve_tx_output)
                .plutus_script_inline_datum(mve_witness, vec![].into())
                .unwrap();
        change_output_creator.add_input(&mve_input_builder);
        tx_builder.add_input(mve_input_builder).unwrap();

        if mve_in_ix == 0 {
            tx_builder.set_exunits(
                RedeemerWitnessKey::new(cml_chain::plutus::RedeemerTag::Spend, mve_in_ix as u64),
                cml_chain::plutus::ExUnits::from(MAKE_VOTING_ESCROW_EX_UNITS),
            );

            tx_builder.set_exunits(
                RedeemerWitnessKey::new(cml_chain::plutus::RedeemerTag::Spend, ve_factory_in_ix as u64),
                cml_chain::plutus::ExUnits::from(VE_FACTORY_EX_UNITS),
            );
        } else {
            tx_builder.set_exunits(
                RedeemerWitnessKey::new(cml_chain::plutus::RedeemerTag::Spend, ve_factory_in_ix as u64),
                cml_chain::plutus::ExUnits::from(VE_FACTORY_EX_UNITS),
            );
            tx_builder.set_exunits(
                RedeemerWitnessKey::new(cml_chain::plutus::RedeemerTag::Spend, mve_in_ix as u64),
                cml_chain::plutus::ExUnits::from(MAKE_VOTING_ESCROW_EX_UNITS),
            );
        }

        let total_num_mints = voting_escrow_value.multiasset.len() + 1;

        // Mint ve_composition tokens --------------------------------------------------------------
        let mint_ve_composition_token_witness = PartialPlutusWitness::new(
            PlutusScriptWitness::Ref(self.ctx.select::<MintVECompositionPolicy>().0),
            cml_chain::plutus::PlutusData::new_integer(BigInteger::from(ve_factory_in_ix)),
        );

        for (_, names) in voting_escrow_value.multiasset.iter() {
            for (asset_name, qty) in names.iter() {
                let mint_ve_composition_builder_result =
                    SingleMintBuilder::new_single_asset(asset_name.clone(), *qty as i64)
                        .plutus_script(mint_ve_composition_token_witness.clone(), vec![].into());
                tx_builder.add_mint(mint_ve_composition_builder_result).unwrap();
            }
        }

        // NOW it is safe to add GT tokens to voting_escrow
        voting_escrow_value.add_unsafe(gt_ac, ve_composition_qty);

        // Mint ve_identifier token ----------------------------------------------------------------
        let mint_identifier_policy = self.ctx.select::<MintVEIdentifierPolicy>().0;
        let mint_ve_identifier_token_witness = PartialPlutusWitness::new(
            PlutusScriptWitness::Ref(mint_identifier_policy),
            ve_factory_output_ref.into_pd(),
        );
        trace!(
            "make_voting_escrow(): ve_factory_in output_ref: {}",
            ve_factory_output_ref
        );
        let mint_ve_identifier_name = compute_identifier_token_asset_name(ve_factory_output_ref);
        trace!(
            "make_voting_escrow(): identifier name: {}",
            mint_ve_identifier_name.to_raw_hex()
        );
        let mint_ve_identifier_builder_result =
            SingleMintBuilder::new_single_asset(mint_ve_identifier_name.clone(), 1)
                .plutus_script(mint_ve_identifier_token_witness.clone(), vec![].into());
        tx_builder.add_mint(mint_ve_identifier_builder_result).unwrap();

        for ix in 0..total_num_mints {
            let ex_units = cml_chain::plutus::ExUnits::from(MVE_TOKEN_MINT_EX_UNITS);
            tx_builder.set_exunits(RedeemerWitnessKey::new(RedeemerTag::Mint, ix as u64), ex_units);
        }

        let id_token = Token(
            mint_identifier_policy,
            spectrum_cardano_lib::AssetName::from(mint_ve_identifier_name.clone()),
        );
        voting_escrow_value.add_unsafe(AssetClass::from(id_token), 1);

        // Add `ve_factory` output -----------------------------------------------------------------
        let network_id = self.ctx.select::<NetworkId>();
        let ve_factory_output = TransactionOutputBuilder::new()
            .with_address(script_address(ve_factory_script_hash, network_id))
            .with_data(ve_factory_datum)
            .next()
            .unwrap()
            .with_asset_and_min_required_coin(ve_factory_out_value.multiasset, COINS_PER_UTXO_BYTE)
            .unwrap()
            .build()
            .unwrap();

        change_output_creator.add_output(&ve_factory_output);
        tx_builder.add_output(ve_factory_output.clone()).unwrap();

        // Add `voting_escrow` output --------------------------------------------------------------
        let ve_datum = order.ve_datum;
        let ve_identifier_name = spectrum_cardano_lib::AssetName::from(mint_ve_identifier_name);
        let next_ve = VotingEscrow {
            gov_token_amount: ve_composition_qty,
            gt_policy: gt_token.policy_id,
            gt_auth_name,
            locked_until: ve_datum.locked_until,
            ve_identifier_name,
            owner: ve_datum.owner,
            max_ex_fee: ve_datum.max_ex_fee,
            version: ve_datum.version,
            last_wp_epoch: ve_datum.last_wp_epoch,
            last_gp_deadline: ve_datum.last_gp_deadline,
        };

        let voting_escrow_datum = DatumOption::new_datum(ve_datum.into_pd());
        voting_escrow_value.coin = mve_coin - 3_000_000;

        let voting_escrow_output = TransactionOutputBuilder::new()
            .with_address(script_address(
                self.ctx.select::<VotingEscrowScriptHash>().0,
                self.ctx.select::<NetworkId>(),
            ))
            .with_data(voting_escrow_datum)
            .next()
            .unwrap()
            .with_value(voting_escrow_value)
            .build()
            .unwrap();

        change_output_creator.add_output(&voting_escrow_output);
        tx_builder.add_output(voting_escrow_output.clone()).unwrap();

        let estimated_tx_fee = tx_builder.min_fee(true).unwrap();
        let actual_fee = estimated_tx_fee + 320_000;

        // TODO: change should be sent to the owner.
        let OperatorCreds(_operator_pkh, operator_addr) = self.ctx.select::<OperatorCreds>();
        let change_output = change_output_creator.create_change_output(actual_fee, operator_addr.clone());
        tx_builder.add_output(change_output).unwrap();
        tx_builder
            .add_collateral(InputBuilderResult::from(self.ctx.select::<Collateral>()))
            .unwrap();
        tx_builder.set_validity_start_interval(current_slot.0);
        tx_builder.set_ttl(current_slot.0 + 300);
        let signed_tx_builder = tx_builder
            .build(ChangeSelectionAlgo::Default, &operator_addr)
            .unwrap();

        let tx_hash = TransactionHash::from_hex(&signed_tx_builder.body().hash().to_hex()).unwrap();

        let add_slot = |output_ref| TimedOutputRef {
            output_ref,
            slot: current_slot,
        };

        let next_ve_factory_version = add_slot(OutputRef::new(tx_hash, 0));
        let fresh_ve_factory = Traced::new(
            Predicted(Bundled(
                Snapshot::new(next_ve_factory, next_ve_factory_version),
                ve_factory_output.output,
            )),
            Some(*ve_factory.version()),
        );
        let next_ve_version = add_slot(OutputRef::new(tx_hash, 1));
        let fresh_ve = Traced::new(
            Predicted(Bundled(
                Snapshot::new(next_ve, next_ve_version),
                voting_escrow_output.output,
            )),
            None,
        );
        Ok((signed_tx_builder, fresh_ve_factory, fresh_ve))
    }
}

fn compute_identifier_token_asset_name(output_ref: OutputRef) -> cml_chain::assets::AssetName {
    use cml_chain::Serialize;
    let mut bytes = output_ref.tx_hash().to_raw_bytes().to_vec();
    bytes.extend_from_slice(
        &cml_chain::plutus::PlutusData::new_integer(BigInteger::from(output_ref.index())).to_cbor_bytes(),
    );
    let token_name = blake2b256(bytes.as_ref());
    cml_chain::assets::AssetName::new(token_name.to_vec()).unwrap()
}

fn script_address(script_hash: ScriptHash, network_id: NetworkId) -> Address {
    EnterpriseAddress::new(u8::from(network_id), StakeCredential::new_script(script_hash)).to_address()
}

/// Here we calculate `cbor.serialise(i)` from Aiken script. The exact calculation that is
/// performed is found here: https://github.com/aiken-lang/aiken/blob/2bb2f11090ace3c7f36ed75b0e1d5b101d0c9a8a/crates/uplc/src/machine/runtime.rs#L1032
fn cbor_serialise_integer(i: u32) -> Vec<u8> {
    let i = uplc_pallas_codec::utils::Int::from(i as i64);

    PlutusData::BigInt(uplc_pallas_primitives::alonzo::BigInt::Int(i))
        .encode_fragment()
        .unwrap()
}

/// Computes index_tn(epoch) from aiken script
pub fn compute_epoch_asset_name(epoch: u32) -> cml_chain::assets::AssetName {
    let bytes = cbor_serialise_integer(epoch);

    let token_name = blake2b256(bytes.as_ref());
    cml_chain::assets::AssetName::new(token_name.to_vec()).unwrap()
}

/// Computes farm_name(farm_id: Int) from aiken script
pub fn compute_farm_name(farm_id: u32) -> cml_chain::assets::AssetName {
    let bytes = cbor_serialise_integer(farm_id);

    cml_chain::assets::AssetName::try_from(bytes).unwrap()
}

// The following enums are used to identify input types for particular TXs after lexicographic
// ordering by TxInput.
enum CreateWPollInputType {
    Inflation,
    WPFactory,
    Funding,
}

enum DistributeInflationInputType {
    WPoll,
    Farm,
    Funding(InputBuilderResult),
}

enum DistributeInflationRefInputType {
    PermManager,
    Other,
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

fn select_funding_boxes<Ctx>(
    target: Coin,
    required_tokens: Vec<BuiltPolicy>,
    boxes: Vec<FundingBox>,
    ctx: &Ctx,
) -> (Vec<InputBuilderResult>, Vec<FundingBox>)
where
    Ctx: Has<OperatorCreds> + Clone,
{
    let mut all_utxos = vec![];
    for funding_box in &boxes {
        let output = funding_box.clone().into_ledger(ctx.clone());
        let output_ref: OutputRef = funding_box.id.into();
        let input = TransactionInput::from(output_ref);
        all_utxos.push(TransactionUnspentOutput::new(input, output));
    }
    let input_results = collect_utxos(all_utxos, target, required_tokens, None);

    let mut selected_boxes = vec![];
    for i in &input_results {
        let output_ref = OutputRef::new(i.input.transaction_id, i.input.index);
        let id = boxes
            .iter()
            .find_map(|f| {
                let funding_output_ref: OutputRef = f.id.into();
                if funding_output_ref == output_ref {
                    Some(f.id)
                } else {
                    None
                }
            })
            .unwrap();
        selected_boxes.push(FundingBox {
            value: i.utxo_info.value().clone(),
            id,
        });
    }
    (input_results, selected_boxes)
}

#[derive(Clone, Debug, Serialize)]
pub enum ExecuteOrderError {
    BadOrMissingInput,
    WeightingExceedsAvailableVotingPower,
    InVotingPower,
    WeightingWitness(WeightingWitnessError),
    Other(String),
}

#[derive(Clone, Debug, Serialize)]
pub enum MakeVotingEscrowError {
    NonAcceptedAsset,
    InsufficientAdaInOrder,
    VEFactoryDatumNotPresent,
    LocktimeExceedsLimit,
}

#[derive(Clone, Debug, Serialize)]
pub enum WeightingWitnessError {
    OwnerAuthFailure,
    VotingEscrowIneligibleToVote {
        last_wp_epoch: i32,
        current_epoch: i32,
    },
    VotingEscrowVersionMismatch {
        voting_escrow_version: u32,
        order_version: u32,
    },
    CannotDecodeRedeemer,
}

#[cfg(test)]
mod tests {
    use cml_chain::{
        address::{Address, BaseAddress, EnterpriseAddress},
        certs::{Credential, StakeCredential},
        transaction::{DatumOption, Transaction},
        Deserialize, Serialize,
    };
    use cml_crypto::{Bip32PrivateKey, ScriptHash};

    use super::compute_mint_wp_auth_token_validator;

    #[test]
    fn test_parametrised_validator() {
        let splash_policy = create_dummy_policy_id(0);
        let farm_auth_policy = create_dummy_policy_id(1);
        let factory_auth_policy = create_dummy_policy_id(2);
        let inflation_box_auth_policy = create_dummy_policy_id(3);
        let zeroth_epoch_start = 100;
        let _ = compute_mint_wp_auth_token_validator(
            splash_policy,
            farm_auth_policy,
            factory_auth_policy,
            inflation_box_auth_policy,
            zeroth_epoch_start,
        );
    }

    #[test]
    fn test_blaze_deployment() {
        let tx_hex = "84a8008282582024da0b6a8037df5b61747b08c061b802054a86e260c2558b0766144cb75b8b6f0182582024da0b6a8037df5b61747b08c061b802054a86e260c2558b0766144cb75b8b6f070183a300581d7038c1745a6f8a6921427f160be2d73bc77d91a7c7703ab2afd29788b701821a02faf080a1581cc72c38ca9933f1fc4b614d1930113a4fd02f3448cd283ed66e189209a142613401028201d818582dd8799f1b00238d7ea4c68000581e581c7bf3980a45756eabfb799fd1998f633176f6d2a2e34de887ddb4e8dbffa300581d7043d7d73cb48e5d7b481ac9197dcb643a941c7685e5ae751eaa96b11601821a00989680a1581c43d7d73cb48e5d7b481ac9197dcb643a941c7685e5ae751eaa96b116a1491b00238d7ea4c6800001028201d818581e581c7bf3980a45756eabfb799fd1998f633176f6d2a2e34de887ddb4e8db825839002e8ec2b01750544eb86070b3085acef5d6983b4d6e1f5cf26e3ec0b84d3464d094658b76fa83cb87317c43440a16e2376b0a826673413daf1b0000000195da142c021a0004d06909a1581c43d7d73cb48e5d7b481ac9197dcb643a941c7685e5ae751eaa96b116a1491b00238d7ea4c68000010b5820fde88b63cd7a03131fac6aab2270fcbef844f274b4884b0b11b53689a17b5d160d8182582024da0b6a8037df5b61747b08c061b802054a86e260c2558b0766144cb75b8b6f0710825839002e8ec2b01750544eb86070b3085acef5d6983b4d6e1f5cf26e3ec0b84d3464d094658b76fa83cb87317c43440a16e2376b0a826673413daf1b00000001967a1ad6128182582059f16714bec4899e2041493f0aa042b2f48f90fc98d1daf9c0e6d900b6989e5c02a3008182582000000000000000000000000000000000000000000000000000000000000000005840000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000582840000d87980821a00d59f801b00000002540be400840100d8799f01ff821a00d59f801b00000002540be4000681590b98590b9501000033323232323232322322322253232323233300b3002300c375400a264a666018646464a66601e600c60206ea80044c8c8c8c8c94ccc050c02cc054dd5007899191919299980c1807980c9baa00113232323232533301d3016301e375400226464646464646464a66604a6038604c6ea80044c8c8c8c94ccc0a4c088c0a8dd500089929998150080a9998150038a9998150020a99981500188008a5014a0294052819b8f300a302e302b37540026eb8c040c0acdd50060b180698151baa0033375e602660526ea8008c07ccc0accdd2a4004660566ea40612f5c06605698103d87a80004bd7019baf374c6602c6eacc034c0a0dd5000a4500374ca66604c603890000a5eb7bdb1804c8c8cc0040052f5bded8c044a66605800226605a66ec0dd480d1ba60034bd6f7b630099191919299981699baf3300c01e0024c0103d8798000133031337606ea4078dd30038028a99981699b8f01e002133031337606ea4078dd300380189981899bb037520046e98004cc01801800cdd598170019bae302c0023030002302e00133330044bd6f7b6300032400400a6054604e6ea800458cc038dd6180498131baa01748008cdd79ba633010012014374c666600297adef6c60003480080088888c8cc004004014894ccc0ac0044cc0b0cdd81ba9005375000897adef6c60132323232533302c3375e6600e01200498103d8798000133030337606ea4024dd40040028a99981619b8f009002133030337606ea4024dd400400189981819bb037520046ea0004cc01801800cdd698168019bae302b002302f002302d00122533302333720004002298103d8798000153330233371e0040022980103d87a800014c103d87b80003001375066e00dd6980618111baa003480088dd9800a99980f180a980f9baa00113232323253330253028002149858dd7181300098130011bad3024001302037540022c6044603e6ea800458c004c078dd51801980f1baa0042302130223022001301032533301b3011301c37540022900009bad3020301d375400264a666036602260386ea80045300103d87a80001323300100137566042603c6ea8008894ccc080004530103d87a8000132323253330203371e91101a400375c604200626030660486ea00052f5c026600a00a0046eb4c084008c090008c088004cc020dd59800980e1baa3001301c375400402c4603e6040002603a60346ea800458cc004dd61801980c9baa00a375a603860326ea8048c0040048894ccc06c0085300103d87a800013232533301a3011003130123301e0024bd70099980280280099b8000348004c07c00cc0740088c0680044c8c8cc004004cc008008cc00c01401c894ccc068004528899299980c19b88375a603a00490000998018018008a50301d00122533301900114bd7009980d180c180d80099801001180e0009119299980b1806180b9baa00114bd6f7b63009bab301b301837540026600600400244646600200200644a666032002298103d87a8000132323253330193371e00c6eb8c06800c4c044cc074dd3000a5eb804cc014014008dd5980d001180e801180d800998009bab30163017301730173017301337540089110022323300100100322533301700114bd6f7b630099191919299980c19b8f007002100313301c337606ea4008dd3000998030030019bab3019003375c602e004603600460320026eb8c050c044dd50008b1809980a001180900098071baa00614984d958c94ccc030c00c0044c8c94ccc044c05000852616375a6024002601c6ea801c54ccc030c00800454ccc03cc038dd50038a4c2c2c60186ea80184cc88c894ccc03cc8c8c94ccc048c020c04cdd5000899191919299980b1806980b9baa0011323232323232533301c3013301d375400226464a66603c602e603e6ea80044c8c8c8c94ccc088cdc780b9bae30273024375400a2a666044008200229405281919192999812180d98129baa00313232533302633300230010070064a226660046466002002600400e44a666056002297ae013232533302a33302a3371e6eb8c06000922011cececc92aeaaac1f5b665f567b01baec8bc2771804b4c21716a87a4e3004a09444cc0b8dd38011980200200089980200200098178011bac302d0010074a229408c8cc004004008894ccc0ac00452f5c0264666444646600200200644a6660620022006264660666e9ccc0ccdd4803198199ba9375c6060002660666ea0dd69818800a5eb80cc00c00cc0d4008c0cc004dd718150009bab302b00133003003302f002302d00122232333001001004002222533302d0021001132323232323330080083035007533302f0061337120026660180140080042940dd69819981a0011bae30320013032002375c60600026eb0c0bc0084c8c94ccc098c074c09cdd500089919192999814981118151baa001132323232533302d3023302e375400226464a666064606a0042646464a66606401c2a6660640162a666064004200229405280a5032330010013758606e6070607060706070607060706070607060686ea8088894ccc0d8004528099299981a198028049bae303900214a22660060060026072002666060660026eb0c068c0c8dd50038012504a244646600200200644a66606c00229404c94ccc0d0cdc79bae303900200414a226600600600260720022c6eb8c0cc004c8cc004004c94ccc0bcc094c0c0dd50008a5eb7bdb1804dd5981a18189baa0013300c00f375c606660606ea8008894ccc0c800452f5c02660666060606800266004004606a0022c6034605c6ea8050dd6180c98169baa00232533302b3022302c3754004264646464a666064606a00426464931980300111bae001330050032375c0022c6eb0c0cc004c0cc008dd6181880098169baa0021622323300100100322533303100114984c8cc00c00cc0d4008c00cc0cc004c0b8c0acdd50008b180a98151baa002301d3330043756602260526ea8004071220101a400301030283754605660506ea800458cc02cdd6180798139baa015375a6054604e6ea8010c8cdd79ba633001006023374c660026eacc03cc09cdd50048119119198008008019129998158008a5eb7bdb1804c8c8c8c94ccc0b0cdc7803801080189981819bb037520046e98004cc01801800cdd598168019bae302b002302f002302d001222325333027301d302837540022900009bad302c3029375400264a66604e603a60506ea8004530103d87a8000132330010013756605a60546ea8008894ccc0b0004530103d87a80001323232533302c3371e00e6eb8c0b400c4c090cc0c0dd4000a5eb804cc014014008dd6981680118180011817000998020018011119198008008019129998148008a60103d87a8000132323253330293371e00c6eb8c0a800c4c084cc0b4dd3000a5eb804cc014014008dd5981500118168011815800980598119baa0153756601460446ea8010dd5980498109baa0083375e00c601860406ea800858c028c07cdd50009810980f1baa0011633001007375a6012603a6ea803cc0040048894ccc07c008530103d87a800013232533301e301500313016330220024bd70099980280280099b8000348004c08c00cc084008c018c068dd50009800980c9baa301c3019375400446038603a0022c6644646600200200644a6660380022980103d87a800013232533301b3375e6012603a6ea80080144c04ccc07c0092f5c02660080080026040004603c0026eb0c00cc05cdd5002980d180b9baa00437586002602c6ea80108c064c068c0680048c06000458c058c05c008c054004c044dd50008a4c26caca66601a6008601c6ea80044c8c8c8c94ccc050c05c0084c926325333012300900115333015301437540042930b0a999809180400089919299980b980d0010a4c2c6eb4c060004c050dd50010b18091baa0011630150013015002375a6026002601e6ea800458dd7003180818069baa005370e90011b8748000dd2a40006e1d2004375c0026eb80055cd2ab9d5573caae7d5d02ba157449811e581c65bb79f9ec437c70413430b7aba049d891b5483a0041deaae98758dd004c011e581cc72c38ca9933f1fc4b614d1930113a4fd02f3448cd283ed66e1892090001f5f6";
        let tx = Transaction::from_cbor_bytes(&hex::decode(tx_hex).unwrap()).unwrap();

        for input in tx.body.inputs {
            println!("input: {}#{}", input.transaction_id.to_hex(), input.index);
        }

        for output in &tx.body.outputs {
            println!(
                "output addr: {}, hex: {}",
                output.address().to_bech32(None).unwrap(),
                output.address().to_hex()
            );
        }
        println!("\n\n");

        let ff_datum = tx.body.outputs[0].datum().unwrap();
        if let DatumOption::Datum {
            datum,
            datum_bytes_encoding,
            ..
        } = ff_datum
        {
            println!("farm_factory datum CBOR: {}", hex::encode(datum.to_cbor_bytes()));
            dbg!(datum);
        } else {
            panic!("");
        }

        println!("\n\n");

        let mut m = tx.body.mint.unwrap();
        for e in m.entries() {
            let sh = e.key();
            let cred = StakeCredential::new_script(*sh);
            let e_addr = EnterpriseAddress::new(0, cred);
            let addr = e_addr.to_address();
            println!("mint script_hash: {}", sh.to_hex());
            println!("mint addr: {}", addr.to_bech32(None).unwrap());
            println!("mint addr (hex): {}", addr.to_hex());
            let vals = e.get();
        }
    }

    #[test]
    fn test_hex_encode() {
        let orig: Vec<u8> = vec![
            102, 97, 114, 109, 48, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
            0, 0,
        ];
        let trunc: Vec<u8> = vec![102, 97, 114, 109, 48];
        let tx_input: Vec<u8> = vec![
            34, 137, 63, 115, 4, 79, 41, 24, 87, 53, 80, 108, 164, 159, 251, 51, 50, 147, 63, 63, 20, 189,
            145, 172, 75, 137, 165, 134, 131, 202, 199, 225,
        ];
        println!("orig: {}", hex::encode(&tx_input));
    }

    fn create_dummy_policy_id(val: u8) -> ScriptHash {
        let bytes: [u8; 28] = std::iter::repeat(val)
            .take(28)
            .collect::<Vec<u8>>()
            .try_into()
            .unwrap();
        ScriptHash::from(bytes)
    }

    #[test]
    fn witness_reward_addr() {
        let weighting_witness_script_hash_hex = "635fcf57b7e16a5f23f18620722688cf2ab227093ef42e49cecf61d1";
        let weighting_witness_script_hash = ScriptHash::from_hex(weighting_witness_script_hash_hex).unwrap();
        let staking_address =
            cml_chain::address::RewardAddress::new(0, Credential::new_script(weighting_witness_script_hash));
        println!("{}", staking_address.to_address().to_bech32(None).unwrap());

        let addr = BaseAddress::new(
            0,
            StakeCredential::new_script(weighting_witness_script_hash),
            StakeCredential::new_script(weighting_witness_script_hash),
        )
        .to_address();
        println!("BASE ADDRESS: {}", addr.to_bech32(None).unwrap());
    }
}
