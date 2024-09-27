use std::time::{SystemTime, UNIX_EPOCH};

use cml_chain::address::Address;
use cml_chain::assets::AssetBundle;
use cml_chain::builders::input_builder::{InputBuilderResult, SingleInputBuilder};
use cml_chain::builders::mint_builder::SingleMintBuilder;
use cml_chain::builders::output_builder::{SingleOutputBuilderResult, TransactionOutputBuilder};
use cml_chain::builders::redeemer_builder::RedeemerWitnessKey;
use cml_chain::builders::tx_builder::{ChangeSelectionAlgo, SignedTxBuilder};
use cml_chain::builders::withdrawal_builder::SingleWithdrawalBuilder;
use cml_chain::builders::witness_builder::{PartialPlutusWitness, PlutusScriptWitness};
use cml_chain::plutus::RedeemerTag;
use cml_chain::transaction::{TransactionInput, TransactionOutput};
use cml_chain::utils::BigInteger;
use cml_chain::{Coin, OrderedHashMap, PolicyId, RequiredSigners};
use cml_crypto::{blake2b256, RawBytesEncoding, TransactionHash};
use spectrum_offchain::data::event::{Predicted, Traced};
use spectrum_offchain_cardano::deployment::DeployedScriptInfo;

use bloom_offchain::execution_engine::bundled::Bundled;
use spectrum_cardano_lib::collateral::Collateral;
use spectrum_cardano_lib::hash::hash_transaction_canonical;
use spectrum_cardano_lib::plutus_data::IntoPlutusData;
use spectrum_cardano_lib::protocol_params::constant_tx_builder;
use spectrum_cardano_lib::transaction::TransactionOutputExtension;
use spectrum_cardano_lib::{AssetClass, AssetName, OutputRef, Token};
use spectrum_offchain::data::Has;
use spectrum_offchain::ledger::{IntoLedger, TryFromLedger};
use uplc::PlutusData;
use uplc_pallas_primitives::Fragment;

use crate::assets::SPLASH_AC;
use crate::constants::{self};
use crate::deployment::ProtocolValidator;
use crate::entities::offchain::voting_order::VotingOrder;
use crate::entities::onchain::funding_box::{FundingBox, FundingBoxId, FundingBoxSnapshot};
use crate::entities::onchain::inflation_box::{unsafe_update_ibox_state, INFLATION_BOX_EX_UNITS};
use crate::entities::onchain::permission_manager::{compute_perm_manager_policy_id, PERM_MANAGER_EX_UNITS};
use crate::entities::onchain::poll_factory::{
    unsafe_update_factory_state, FactoryRedeemer, PollFactoryAction, GOV_PROXY_EX_UNITS, WP_FACTORY_EX_UNITS,
};
use crate::entities::onchain::smart_farm::{self, compute_mint_farm_auth_token_policy_id, FARM_EX_UNITS};
use crate::entities::onchain::voting_escrow::{
    self, compute_mint_weighting_power_policy_id, compute_voting_escrow_policy_id, unsafe_update_ve_state,
    VotingEscrowAction, VotingEscrowAuthorizedAction, ORDER_WITNESS_EX_UNITS, VOTING_ESCROW_EX_UNITS,
    WEIGHTING_POWER_EX_UNITS,
};
use crate::entities::onchain::weighting_poll::{
    self, compute_mint_wp_auth_token_policy_id, unsafe_update_wp_state, MintAction, WeightingPoll,
    MINT_WP_AUTH_EX_UNITS,
};
use crate::entities::Snapshot;
use crate::protocol_config::{
    EDaoMSigAuthPolicy, FactoryAuthPolicy, FarmAuthPolicy, FarmAuthRefScriptOutput, GTAuthPolicy,
    GovProxyRefScriptOutput, InflationAuthPolicy, InflationBoxRefScriptOutput, MintWPAuthPolicy,
    MintWPAuthRefScriptOutput, NodeMagic, OperatorCreds, PermManagerAuthPolicy,
    PermManagerBoxRefScriptOutput, PollFactoryRefScriptOutput, Reward, SplashPolicy, VEFactoryAuthPolicy,
    VotingEscrowPolicy, VotingEscrowRefScriptOutput, WPFactoryAuthPolicy, WeightingPowerPolicy,
    WeightingPowerRefScriptOutput, TX_FEE_CORRECTION,
};
use crate::GenesisEpochStartTime;

use super::{
    AvailableFundingBoxes, FundingBoxChanges, InflationBoxSnapshot, PermManagerSnapshot, PollFactorySnapshot,
    SmartFarmSnapshot, VotingEscrowSnapshot, WeightingPollSnapshot,
};

#[async_trait::async_trait]
pub trait InflationActions<Bearer> {
    async fn create_wpoll(
        &self,
        inflation_box: Bundled<InflationBoxSnapshot, Bearer>,
        factory: Bundled<PollFactorySnapshot, Bearer>,
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
    ) -> SignedTxBuilder;
    async fn execute_order(
        &self,
        weighting_poll: Bundled<WeightingPollSnapshot, Bearer>,
        order: (VotingOrder, Bundled<VotingEscrowSnapshot, Bearer>),
        funding_boxes: AvailableFundingBoxes,
    ) -> (
        SignedTxBuilder,
        Traced<Predicted<Bundled<WeightingPollSnapshot, Bearer>>>,
        Traced<Predicted<Bundled<VotingEscrowSnapshot, Bearer>>>,
    );
    async fn distribute_inflation(
        &self,
        weighting_poll: Bundled<WeightingPollSnapshot, Bearer>,
        farm: Bundled<SmartFarmSnapshot, Bearer>,
        perm_manager: Bundled<PermManagerSnapshot, Bearer>,
        farm_weight: u64,
        funding_boxes: AvailableFundingBoxes,
    ) -> (
        SignedTxBuilder,
        Traced<Predicted<Bundled<WeightingPollSnapshot, Bearer>>>,
        Traced<Predicted<Bundled<SmartFarmSnapshot, Bearer>>>,
        Traced<Predicted<Bundled<PermManagerSnapshot, Bearer>>>,
    );
}

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
        + Has<FarmAuthPolicy>
        + Has<FarmAuthRefScriptOutput>
        + Has<FactoryAuthPolicy>
        + Has<WPFactoryAuthPolicy>
        + Has<VEFactoryAuthPolicy>
        + Has<VotingEscrowRefScriptOutput>
        + Has<WeightingPowerPolicy>
        + Has<WeightingPowerRefScriptOutput>
        + Has<PermManagerBoxRefScriptOutput>
        + Has<GovProxyRefScriptOutput>
        + Has<EDaoMSigAuthPolicy>
        + Has<PermManagerAuthPolicy>
        + Has<GTAuthPolicy>
        + Has<NodeMagic>
        + Has<OperatorCreds>
        + Has<GenesisEpochStartTime>
        + Has<DeployedScriptInfo<{ ProtocolValidator::GovProxy as u8 }>>,
{
    async fn create_wpoll(
        &self,
        Bundled(inflation_box, inflation_box_in): Bundled<InflationBoxSnapshot, TransactionOutput>,
        Bundled(factory, factory_in): Bundled<PollFactorySnapshot, TransactionOutput>,
        funding_boxes: AvailableFundingBoxes,
    ) -> (
        SignedTxBuilder,
        Traced<Predicted<Bundled<InflationBoxSnapshot, TransactionOutput>>>,
        Traced<Predicted<Bundled<PollFactorySnapshot, TransactionOutput>>>,
        Traced<Predicted<Bundled<WeightingPollSnapshot, TransactionOutput>>>,
        FundingBoxChanges,
    ) {
        let mut tx_builder = constant_tx_builder();

        // Set TX validity range
        let current_posix_time = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();
        let start_slot = 68004028; //67580376;
        tx_builder.set_validity_start_interval(start_slot);
        tx_builder.set_ttl(start_slot + 43200);

        let wpoll_auth_policy = self.ctx.select::<MintWPAuthPolicy>().0;
        let splash_policy = self.ctx.select::<SplashPolicy>().0;
        let genesis_time = self.ctx.select::<GenesisEpochStartTime>().0;
        let farm_auth_policy = self.ctx.select::<FarmAuthPolicy>().0;

        // Note that we're not actually minting weighting power here. We only need the minting
        // policy id as part of the inflation box's script.
        //let weighting_power_policy = compute_mint_weighting_power_policy_id(
        //self.ctx.select::<GenesisEpochStartTime>().0,
        //wpoll_auth_policy,
        //self.ctx.select::<GTAuthPolicy>().0,
        //);

        let inflation_script_hash = self
            .ctx
            .select::<DeployedScriptInfo<{ ProtocolValidator::Inflation as u8 }>>()
            .script_hash;
        println!("inflation_script_hash: {}", inflation_script_hash.to_hex());
        let inflation_script = PartialPlutusWitness::new(
            PlutusScriptWitness::Ref(inflation_script_hash),
            cml_chain::plutus::PlutusData::Integer(BigInteger::from(0)),
        );

        let inflation_input = SingleInputBuilder::new(
            TransactionInput::from(*(inflation_box.version())),
            inflation_box_in.clone(),
        )
        .plutus_script_inline_datum(inflation_script, RequiredSigners::from(vec![]))
        .unwrap();

        tx_builder.add_reference_input(self.ctx.select::<InflationBoxRefScriptOutput>().0.clone());

        let prev_ib_version = *inflation_box.version();
        let (next_inflation_box, emission_rate) = inflation_box.get().release_next_tranche();
        println!(
            "inflation::release_next_tranche --> emission_rate: {}",
            emission_rate.untag()
        );
        let mut inflation_box_out = inflation_box_in.clone();
        if let Some(data_mut) = inflation_box_out.data_mut() {
            unsafe_update_ibox_state(data_mut, next_inflation_box.last_processed_epoch);
        }
        inflation_box_out.sub_asset(*SPLASH_AC, emission_rate.untag());
        let inflation_output = SingleOutputBuilderResult::new(inflation_box_out.clone());
        tx_builder.add_output(inflation_output).unwrap();

        // WP factory

        let wp_factory_script_hash = self
            .ctx
            .select::<DeployedScriptInfo<{ ProtocolValidator::WpFactory as u8 }>>()
            .script_hash;
        println!("wp_factory_script_hash: {}", wp_factory_script_hash.to_hex());

        let factory_redeemer = FactoryRedeemer {
            successor_ix: 2,
            action: PollFactoryAction::CreatePoll,
        };
        let wp_factory_script = PartialPlutusWitness::new(
            PlutusScriptWitness::Ref(wp_factory_script_hash),
            factory_redeemer.into_pd(),
        );

        let wp_factory_input =
            SingleInputBuilder::new(TransactionInput::from(*(factory.version())), factory_in.clone())
                .plutus_script_inline_datum(wp_factory_script, RequiredSigners::from(vec![]))
                .unwrap();

        tx_builder.add_reference_input(self.ctx.select::<PollFactoryRefScriptOutput>().0.clone());

        let prev_factory_version = *factory.version();
        let (next_factory, fresh_wpoll) = factory.unwrap().next_weighting_poll(emission_rate);
        let mut factory_out = factory_in;
        if let Some(data_mut) = factory_out.data_mut() {
            println!("INPUT WP_FACTORY DATUM: {:?}", data_mut);
            unsafe_update_factory_state(data_mut, next_factory.last_poll_epoch.unwrap());
        }

        let (input_results, funding_boxes_to_spend) =
            select_funding_boxes(5_000_000, funding_boxes.0, &self.ctx);

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

        // Mint wp_auth token TODO: don't need to compute, it's in deployment
        let mint_wp_auth_token_script_hash = compute_mint_wp_auth_token_policy_id(
            splash_policy,
            farm_auth_policy,
            self.ctx.select::<WPFactoryAuthPolicy>().0,
            self.ctx.select::<InflationAuthPolicy>().0,
            genesis_time,
        );
        println!(
            "mint_wp_auth_token_script_hash: {}",
            mint_wp_auth_token_script_hash.to_hex()
        );
        let mint_wp_auth_token_witness = PartialPlutusWitness::new(
            PlutusScriptWitness::Ref(mint_wp_auth_token_script_hash),
            mint_action.into_pd(),
        );
        let OperatorCreds(operator_pkh, _operator_addr) = self.ctx.select::<OperatorCreds>();

        println!("operator_addr: {:?}", _operator_addr.to_bech32(None));
        println!("operator_pkh: {}", operator_pkh.to_hex());
        println!(
            "inflation_box.last_processed_epoch: {}",
            inflation_box.get().last_processed_epoch
        );
        // Compute index_tn(epoch), where `epoch` is the current epoch
        //let asset = compute_epoch_asset_name(inflation_box.get().last_processed_epoch);
        let asset = compute_epoch_asset_name(0);
        println!("mint_wp_auth_token name: {}", hex::encode(&asset.inner));
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
        let ord_hash_map = OrderedHashMap::from_iter(vec![(mint_wp_auth_token_script_hash, asset_pair)]);
        match &mut wpoll_out {
            TransactionOutput::AlonzoFormatTxOut(tx_out) => {
                let multiasset = tx_out
                    .amount
                    .multiasset
                    .checked_add(&AssetBundle::from(ord_hash_map))
                    .unwrap();
                tx_out.amount.multiasset = multiasset;
                println!("AlonzoFormatTxOut coin: {}", tx_out.amount.coin);
            }

            TransactionOutput::ConwayFormatTxOut(tx_out) => {
                let multiasset = tx_out
                    .amount
                    .multiasset
                    .checked_add(&AssetBundle::from(ord_hash_map))
                    .unwrap();
                tx_out.amount.multiasset = multiasset;
                println!("ConwayFormatTxOut coin: {}", tx_out.amount.coin);
            }
        }

        let weighting_poll_output = SingleOutputBuilderResult::new(wpoll_out.clone());
        tx_builder.add_output(weighting_poll_output).unwrap();

        let factory_output = SingleOutputBuilderResult::new(factory_out.clone());
        tx_builder.add_output(factory_output).unwrap();

        // Set Governance Proxy witness script
        let OperatorCreds(_, operator_address) = self.ctx.select::<OperatorCreds>();

        //let gov_witness_script_hash = self
        //    .ctx
        //    .select::<DeployedScriptInfo<{ ProtocolValidator::GovProxy as u8 }>>()
        //    .script_hash;
        //let gp_witness = PartialPlutusWitness::new(
        //    PlutusScriptWitness::Ref(gov_witness_script_hash),
        //    cml_chain::plutus::PlutusData::new_list(vec![]), // dummy value (this validator doesn't require redeemer)
        //);
        //let withdrawal_result = SingleWithdrawalBuilder::new(operator_address.clone(), 0)
        //    .plutus_script(gp_witness, vec![])
        //    .unwrap();
        //tx_builder.add_reference_input(self.ctx.select::<GovProxyRefScriptOutput>().0.clone());
        //tx_builder.add_withdrawal(withdrawal_result);
        //tx_builder.set_exunits(
        //    RedeemerWitnessKey::new(RedeemerTag::Reward, 0),
        //    GOV_PROXY_EX_UNITS,
        //);

        tx_builder
            .add_collateral(InputBuilderResult::from(self.ctx.select::<Collateral>()))
            .unwrap();

        let estimated_tx_fee = tx_builder.min_fee(true).unwrap();
        tx_builder.set_fee(estimated_tx_fee + TX_FEE_CORRECTION);

        // Build tx, change is execution fee.
        let signed_tx_builder = tx_builder
            .build(ChangeSelectionAlgo::Default, &operator_address)
            .unwrap();
        let tx_body = signed_tx_builder.body();

        let tx_hash = hash_transaction_canonical(&tx_body);

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

        let next_ib_version = OutputRef::new(tx_hash, 0);
        let next_traced_ibox = Traced::new(
            Predicted(Bundled(
                Snapshot::new(next_inflation_box, next_ib_version),
                inflation_box_out.clone(),
            )),
            Some(prev_ib_version),
        );
        let fresh_wpoll_version = OutputRef::new(tx_hash, 1);
        let fresh_wpoll = Traced::new(
            Predicted(Bundled(
                Snapshot::new(fresh_wpoll, fresh_wpoll_version),
                wpoll_out,
            )),
            None,
        );
        let next_factory_version = OutputRef::new(tx_hash, 2);
        let next_traced_factory = Traced::new(
            Predicted(Bundled(
                Snapshot::new(next_factory, next_factory_version),
                factory_out.clone(),
            )),
            Some(prev_factory_version),
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
    ) -> SignedTxBuilder {
        let mut tx_builder = constant_tx_builder();

        let splash_policy = self.ctx.select::<SplashPolicy>().0;
        let genesis_time = self.ctx.select::<GenesisEpochStartTime>().0;
        let farm_auth_policy = self.ctx.select::<FarmAuthPolicy>().0;
        let factory_auth_policy = self.ctx.select::<FactoryAuthPolicy>().0;
        let inflation_box_auth_policy = self.ctx.select::<InflationAuthPolicy>().0;
        let wpoll_auth_ref_script = self.ctx.select::<MintWPAuthRefScriptOutput>().0;

        let weighting_poll_script_hash = compute_mint_wp_auth_token_policy_id(
            splash_policy,
            farm_auth_policy,
            factory_auth_policy,
            inflation_box_auth_policy,
            genesis_time,
        );

        let redeemer = weighting_poll::PollAction::Destroy;
        let weighting_poll_script = PartialPlutusWitness::new(
            PlutusScriptWitness::Ref(weighting_poll_script_hash),
            redeemer.into_pd(),
        );

        let weighting_poll_input = SingleInputBuilder::new(
            TransactionInput::from(*(weighting_poll.version())),
            weighting_poll_in.clone(),
        )
        .plutus_script_inline_datum(weighting_poll_script, RequiredSigners::from(vec![]))
        .unwrap();

        let mut output_value = match weighting_poll_in {
            TransactionOutput::AlonzoFormatTxOut(tx) => tx.amount.clone(),
            TransactionOutput::ConwayFormatTxOut(tx) => tx.amount.clone(),
        };

        let estimated_tx_fee = tx_builder.min_fee(true).unwrap() + TX_FEE_CORRECTION;
        tx_builder.set_fee(estimated_tx_fee);
        if estimated_tx_fee > output_value.coin {
            panic!("Not enough ADA in weighting_poll");
        }
        output_value.coin -= estimated_tx_fee;

        tx_builder.add_reference_input(wpoll_auth_ref_script);
        tx_builder.add_input(weighting_poll_input).unwrap();

        let OperatorCreds(_, operator_addr) = self.ctx.select::<OperatorCreds>();
        let output = TransactionOutputBuilder::new()
            .with_address(operator_addr)
            .next()
            .unwrap()
            .with_value(output_value)
            .build()
            .unwrap();
        tx_builder.add_output(output).unwrap();
        tx_builder.set_exunits(
            RedeemerWitnessKey::new(RedeemerTag::Spend, 0),
            MINT_WP_AUTH_EX_UNITS,
        );

        let execution_fee_address: Address = self.ctx.select::<Reward>().0.clone().into();
        tx_builder
            .build(ChangeSelectionAlgo::Default, &execution_fee_address)
            .unwrap()
    }

    async fn execute_order(
        &self,
        Bundled(weighting_poll, weighting_poll_in): Bundled<WeightingPollSnapshot, TransactionOutput>,
        (order, Bundled(voting_escrow, ve_box_in)): (
            VotingOrder,
            Bundled<VotingEscrowSnapshot, TransactionOutput>,
        ),
        funding_boxes: AvailableFundingBoxes,
    ) -> (
        SignedTxBuilder,
        Traced<Predicted<Bundled<WeightingPollSnapshot, TransactionOutput>>>,
        Traced<Predicted<Bundled<VotingEscrowSnapshot, TransactionOutput>>>,
    ) {
        let mut tx_builder = constant_tx_builder();

        let prev_ve_version = voting_escrow.version();
        let prev_wp_version = weighting_poll.version();

        // Voting escrow
        let mut voting_escrow_out = ve_box_in.clone();
        if let Some(data_mut) = voting_escrow_out.data_mut() {
            unsafe_update_ve_state(data_mut, weighting_poll.get().epoch);
        }

        let authorized_action = VotingEscrowAuthorizedAction {
            action: VotingEscrowAction::Governance,
            witness: order.witness,
            version: order.version,
            signature: order.proof,
        };

        let genesis_time = self.ctx.select::<GenesisEpochStartTime>().0;
        let farm_auth_policy = self.ctx.select::<FarmAuthPolicy>().0;
        let splash_policy = self.ctx.select::<SplashPolicy>().0;
        let ve_factory_auth_policy = self.ctx.select::<VEFactoryAuthPolicy>().0;
        let voting_escrow_ref_script = self.ctx.select::<VotingEscrowRefScriptOutput>().0;
        let wpoll_auth_policy = self.ctx.select::<MintWPAuthPolicy>().0;
        let factory_auth_policy = self.ctx.select::<FactoryAuthPolicy>().0;
        let inflation_box_auth_policy = self.ctx.select::<InflationAuthPolicy>().0;
        let wpoll_auth_ref_script = self.ctx.select::<MintWPAuthRefScriptOutput>().0;
        let weighting_power_ref_script = self.ctx.select::<WeightingPowerRefScriptOutput>().0;

        let voting_escrow_script_hash = compute_voting_escrow_policy_id(ve_factory_auth_policy);
        let voting_escrow_script = PartialPlutusWitness::new(
            PlutusScriptWitness::Ref(voting_escrow_script_hash),
            authorized_action.into_pd(),
        );

        let voting_escrow_input = SingleInputBuilder::new(
            TransactionInput::from(*(voting_escrow.version())),
            ve_box_in.clone(),
        )
        .plutus_script_inline_datum(voting_escrow_script, RequiredSigners::from(vec![]))
        .unwrap();

        let voting_escrow_input_tx_hash = voting_escrow_input.input.transaction_id;

        tx_builder.add_reference_input(voting_escrow_ref_script);
        tx_builder.add_input(voting_escrow_input).unwrap();

        // weighting_poll
        let weighting_poll_script_hash = compute_mint_wp_auth_token_policy_id(
            splash_policy,
            farm_auth_policy,
            factory_auth_policy,
            inflation_box_auth_policy,
            genesis_time,
        );
        let weighting_poll_script = PartialPlutusWitness::new(
            PlutusScriptWitness::Ref(weighting_poll_script_hash),
            weighting_poll::PollAction::Vote.into_pd(),
        );

        let weighting_poll_input = SingleInputBuilder::new(
            TransactionInput::from(*(weighting_poll.version())),
            weighting_poll_in.clone(),
        )
        .plutus_script_inline_datum(weighting_poll_script, RequiredSigners::from(vec![]))
        .unwrap();

        let weighting_poll_input_tx_hash = weighting_poll_input.input.transaction_id;

        tx_builder.add_reference_input(wpoll_auth_ref_script);
        tx_builder.add_input(weighting_poll_input).unwrap();

        // Compute the policy for `mint_weighting_power`, to allow us to add the weighting power to WeightingPoll's
        // UTxO.
        let mint_weighting_power_policy = compute_mint_weighting_power_policy_id(
            self.ctx.select::<GenesisEpochStartTime>().0,
            wpoll_auth_policy,
            voting_escrow.get().gt_policy,
        );
        let weighting_power_asset_name = compute_epoch_asset_name(weighting_poll.get().epoch);
        let current_posix_time = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();

        let mut wpoll_out = weighting_poll_in.clone();
        if let Some(data_mut) = wpoll_out.data_mut() {
            unsafe_update_wp_state(data_mut, &order.distribution);
        }
        let weighting_power = voting_escrow.get().voting_power(current_posix_time);
        wpoll_out.add_asset(
            spectrum_cardano_lib::AssetClass::Token(Token(
                mint_weighting_power_policy,
                AssetName::from(weighting_power_asset_name.clone()),
            )),
            weighting_power,
        );

        let next_weighting_poll = WeightingPoll {
            distribution: order.distribution,
            weighting_power: Some(weighting_power),
            ..weighting_poll.get().clone()
        };
        // Set TX outputs
        let weighting_poll_output = SingleOutputBuilderResult::new(wpoll_out.clone());
        tx_builder.add_output(weighting_poll_output).unwrap();

        // The contract requires voting_escrow_out has index 1
        let voting_escrow_output = SingleOutputBuilderResult::new(voting_escrow_out.clone());
        tx_builder.add_output(voting_escrow_output).unwrap();

        // Mint weighting power
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

        // Set witness script (needed by voting_escrow script)
        let reward_address = self.ctx.select::<Reward>().0;
        let order_witness = PartialPlutusWitness::new(
            PlutusScriptWitness::Ref(order.witness),
            cml_chain::plutus::PlutusData::new_list(vec![]), // dummy value (this validator doesn't require redeemer)
        );
        let withdrawal_result = SingleWithdrawalBuilder::new(reward_address.clone(), 0)
            .plutus_script(order_witness, RequiredSigners::from(vec![]))
            .unwrap();
        tx_builder.add_withdrawal(withdrawal_result);
        tx_builder.set_exunits(
            RedeemerWitnessKey::new(RedeemerTag::Reward, 0),
            ORDER_WITNESS_EX_UNITS,
        );

        // Set TX validity range
        tx_builder.set_validity_start_interval(current_posix_time);
        tx_builder.set_ttl(constants::MAX_TIME_DRIFT_MILLIS);

        tx_builder
            .add_collateral(InputBuilderResult::from(self.ctx.select::<Collateral>()))
            .unwrap();

        let estimated_tx_fee = tx_builder.min_fee(true).unwrap();
        tx_builder.set_fee(estimated_tx_fee + TX_FEE_CORRECTION);

        let execution_fee_address: Address = reward_address.into();

        // Build tx, change is execution fee.
        let signed_tx_builder = tx_builder
            .build(ChangeSelectionAlgo::Default, &execution_fee_address)
            .unwrap();
        let tx_body = signed_tx_builder.body();

        let tx_hash = hash_transaction_canonical(&tx_body);

        let next_wp_version = OutputRef::new(tx_hash, 0);
        let fresh_wp = Traced::new(
            Predicted(Bundled(
                Snapshot::new(next_weighting_poll, next_wp_version),
                wpoll_out,
            )),
            Some(*prev_wp_version),
        );

        let next_ve_version = OutputRef::new(tx_hash, 1);
        let next_ve = *voting_escrow.get();
        let fresh_ve = Traced::new(
            Predicted(Bundled(
                Snapshot::new(next_ve, next_ve_version),
                voting_escrow_out,
            )),
            Some(*prev_ve_version),
        );

        (signed_tx_builder, fresh_wp, fresh_ve)
    }

    async fn distribute_inflation(
        &self,
        Bundled(weighting_poll, weighting_poll_in): Bundled<WeightingPollSnapshot, TransactionOutput>,
        Bundled(farm, farm_in): Bundled<SmartFarmSnapshot, TransactionOutput>,
        Bundled(perm_manager, perm_manager_in): Bundled<PermManagerSnapshot, TransactionOutput>,
        farm_weight: u64,
        funding_boxes: AvailableFundingBoxes,
    ) -> (
        SignedTxBuilder,
        Traced<Predicted<Bundled<WeightingPollSnapshot, TransactionOutput>>>,
        Traced<Predicted<Bundled<SmartFarmSnapshot, TransactionOutput>>>,
        Traced<Predicted<Bundled<PermManagerSnapshot, TransactionOutput>>>,
    ) {
        let mut tx_builder = constant_tx_builder();

        let genesis_time = self.ctx.select::<GenesisEpochStartTime>().0;
        let farm_auth_policy = self.ctx.select::<FarmAuthPolicy>().0;
        let splash_policy = self.ctx.select::<SplashPolicy>().0;
        let factory_auth_policy = self.ctx.select::<FactoryAuthPolicy>().0;
        let inflation_box_auth_policy = self.ctx.select::<InflationAuthPolicy>().0;
        let wpoll_auth_ref_script = self.ctx.select::<MintWPAuthRefScriptOutput>().0;
        let smart_farm_ref_script = self.ctx.select::<FarmAuthRefScriptOutput>().0;
        let edao_msig_policy = self.ctx.select::<EDaoMSigAuthPolicy>().0;
        let perm_manager_auth_policy = self.ctx.select::<PermManagerAuthPolicy>().0;
        let perm_manager_box_ref_script = self.ctx.select::<PermManagerBoxRefScriptOutput>().0;

        let mut next_weighting_poll = weighting_poll.get().clone();
        let ix = next_weighting_poll
            .distribution
            .iter()
            .position(|&(farm_id, _)| farm_id == farm.get().farm_id)
            .unwrap();
        let old_weight = next_weighting_poll.distribution[ix].1;
        next_weighting_poll.distribution[ix].1 = old_weight + farm_weight;

        let weighting_poll_script_hash = compute_mint_wp_auth_token_policy_id(
            splash_policy,
            farm_auth_policy,
            factory_auth_policy,
            inflation_box_auth_policy,
            genesis_time,
        );

        // Setting TX inputs
        enum InputType {
            WPoll,
            Farm,
            PermManager,
        }

        let mut typed_inputs = vec![
            (
                TransactionInput::from(*weighting_poll.version()).transaction_id,
                InputType::WPoll,
            ),
            (
                TransactionInput::from(*farm.version()).transaction_id,
                InputType::Farm,
            ),
            (
                TransactionInput::from(*perm_manager.version()).transaction_id,
                InputType::PermManager,
            ),
        ];

        typed_inputs.sort_by_key(|(tx_hash, _)| *tx_hash);
        let farm_in_ix = typed_inputs
            .iter()
            .position(|(_, t)| matches!(t, InputType::Farm))
            .unwrap() as u32;
        let perm_manager_input_ix = typed_inputs
            .iter()
            .position(|(_, t)| matches!(t, InputType::PermManager))
            .unwrap() as u32;

        for (i, (_, input_type)) in typed_inputs.into_iter().enumerate() {
            match input_type {
                InputType::WPoll => {
                    let redeemer = weighting_poll::PollAction::Distribute {
                        farm_ix: 1,
                        farm_in_ix,
                    };
                    let weighting_poll_script = PartialPlutusWitness::new(
                        PlutusScriptWitness::Ref(weighting_poll_script_hash),
                        redeemer.into_pd(),
                    );

                    let weighting_poll_input = SingleInputBuilder::new(
                        TransactionInput::from(*(weighting_poll.version())),
                        weighting_poll_in.clone(),
                    )
                    .plutus_script_inline_datum(weighting_poll_script, RequiredSigners::from(vec![]))
                    .unwrap();
                    tx_builder.add_reference_input(wpoll_auth_ref_script.clone());
                    tx_builder.add_input(weighting_poll_input).unwrap();
                    tx_builder.set_exunits(
                        RedeemerWitnessKey::new(RedeemerTag::Spend, i as u64),
                        MINT_WP_AUTH_EX_UNITS,
                    );
                }
                InputType::Farm => {
                    let redeemer = smart_farm::Redeemer {
                        successor_out_ix: 1,
                        action: smart_farm::Action::DistributeRewards {
                            perm_manager_input_ix,
                        },
                    };
                    let farm_auth_policy = self.ctx.select::<FarmAuthPolicy>().0.to_hex();
                    let smart_farm_script_hash = compute_mint_farm_auth_token_policy_id(
                        &farm_auth_policy,
                        splash_policy,
                        factory_auth_policy,
                    );
                    let smart_farm_script = PartialPlutusWitness::new(
                        PlutusScriptWitness::Ref(smart_farm_script_hash),
                        redeemer.into_pd(),
                    );

                    let smart_farm_input = SingleInputBuilder::new(
                        TransactionInput::from(*(farm.version())),
                        weighting_poll_in.clone(),
                    )
                    .plutus_script_inline_datum(smart_farm_script, RequiredSigners::from(vec![]))
                    .unwrap();
                    tx_builder.add_reference_input(smart_farm_ref_script.clone());
                    tx_builder.add_input(smart_farm_input).unwrap();
                    tx_builder.set_exunits(
                        RedeemerWitnessKey::new(RedeemerTag::Spend, i as u64),
                        FARM_EX_UNITS,
                    );
                }
                InputType::PermManager => {
                    let perm_manager_script_hash =
                        compute_perm_manager_policy_id(edao_msig_policy, perm_manager_auth_policy);

                    let perm_manager_script = PartialPlutusWitness::new(
                        PlutusScriptWitness::Ref(perm_manager_script_hash),
                        cml_chain::plutus::PlutusData::Integer(BigInteger::from(2)), // set successor_out_ix to 2
                    );

                    let perm_manager_input = SingleInputBuilder::new(
                        TransactionInput::from(*(perm_manager.version())),
                        weighting_poll_in.clone(),
                    )
                    .plutus_script_inline_datum(perm_manager_script, RequiredSigners::from(vec![]))
                    .unwrap();
                    tx_builder.add_reference_input(perm_manager_box_ref_script.clone());
                    tx_builder.add_input(perm_manager_input).unwrap();
                    tx_builder.set_exunits(
                        RedeemerWitnessKey::new(RedeemerTag::Spend, i as u64),
                        PERM_MANAGER_EX_UNITS,
                    );
                }
            }
        }

        // Adjust splash values in weighting_poll and farm.
        let splash_emission = weighting_poll.get().emission_rate.untag() * farm_weight
            / weighting_poll.get().weighting_power.unwrap();

        let mut weighting_poll_out = weighting_poll_in.clone();
        weighting_poll_out.sub_asset(*SPLASH_AC, splash_emission);

        let mut farm_out = farm_in.clone();
        farm_out.add_asset(*SPLASH_AC, splash_emission);

        // farm output must be at index 1
        let weighting_poll_output = SingleOutputBuilderResult::new(weighting_poll_out.clone());
        let farm_output = SingleOutputBuilderResult::new(farm_out.clone());
        let perm_manager_output = SingleOutputBuilderResult::new(perm_manager_in.clone());
        tx_builder.add_output(weighting_poll_output).unwrap();
        tx_builder.add_output(farm_output).unwrap();
        tx_builder.add_output(perm_manager_output).unwrap();

        // Add operator as signatory
        let OperatorCreds(operator_pkh, _) = self.ctx.select::<OperatorCreds>();
        tx_builder.add_required_signer(operator_pkh);

        tx_builder
            .add_collateral(InputBuilderResult::from(self.ctx.select::<Collateral>()))
            .unwrap();

        let estimated_tx_fee = tx_builder.min_fee(true).unwrap();
        tx_builder.set_fee(estimated_tx_fee + TX_FEE_CORRECTION);

        let execution_fee_address: Address = self.ctx.select::<Reward>().0.into();

        // Build tx, change is execution fee.
        let signed_tx_builder = tx_builder
            .build(ChangeSelectionAlgo::Default, &execution_fee_address)
            .unwrap();
        let tx_body = signed_tx_builder.body();

        let tx_hash = hash_transaction_canonical(&tx_body);

        let next_wp_version = OutputRef::new(tx_hash, 0);
        let fresh_wp = Traced::new(
            Predicted(Bundled(
                Snapshot::new(next_weighting_poll, next_wp_version),
                weighting_poll_out,
            )),
            Some(*weighting_poll.version()),
        );

        let next_farm_version = OutputRef::new(tx_hash, 1);
        let next_farm = farm.get().clone();
        let fresh_farm = Traced::new(
            Predicted(Bundled(Snapshot::new(next_farm, next_farm_version), farm_out)),
            Some(*farm.version()),
        );

        let next_perm_manager_version = OutputRef::new(tx_hash, 2);
        let next_perm_manager = perm_manager.get().clone();
        let fresh_perm_manager = Traced::new(
            Predicted(Bundled(
                Snapshot::new(next_perm_manager, next_perm_manager_version),
                perm_manager_in,
            )),
            Some(*perm_manager.version()),
        );

        (signed_tx_builder, fresh_wp, fresh_farm, fresh_perm_manager)
    }
}

/// Computes index_tn(epoch) from aiken script
pub fn compute_epoch_asset_name(epoch: u32) -> cml_chain::assets::AssetName {
    let i = uplc_pallas_codec::utils::Int::from(epoch as i64);

    // Here we calculate `cbor.serialise(i)` from Aiken script. The exact calculation that is
    // performed is found here: https://github.com/aiken-lang/aiken/blob/2bb2f11090ace3c7f36ed75b0e1d5b101d0c9a8a/crates/uplc/src/machine/runtime.rs#L1032
    let bytes = PlutusData::BigInt(uplc_pallas_primitives::alonzo::BigInt::Int(i))
        .encode_fragment()
        .unwrap();

    let token_name = blake2b256(bytes.as_ref());
    cml_chain::assets::AssetName::new(token_name.to_vec()).unwrap()
}

enum CreateWPollInputType {
    Inflation,
    WPFactory,
    Funding,
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
    boxes: Vec<FundingBox>,
    ctx: &Ctx,
) -> (Vec<InputBuilderResult>, Vec<FundingBox>)
where
    Ctx: Has<OperatorCreds> + Clone,
{
    let mut total_coin = 0;
    let mut input_results = vec![];
    let mut selected_boxes = vec![];
    for funding_box in boxes {
        let coin = funding_box.value.coin;
        total_coin += coin;
        if total_coin >= target {
            let output = funding_box.clone().into_ledger(ctx.clone());
            let output_ref: OutputRef = funding_box.id.into();
            let input = SingleInputBuilder::new(TransactionInput::from(output_ref), output)
                .payment_key()
                .unwrap();
            input_results.push(input);
            selected_boxes.push(funding_box);
            break;
        }
    }
    (input_results, selected_boxes)
}

#[cfg(test)]
mod tests {
    use cml_chain::{
        address::{Address, EnterpriseAddress},
        certs::StakeCredential,
        transaction::{DatumOption, Transaction},
        Deserialize,
    };
    use cml_crypto::ScriptHash;

    use super::compute_mint_wp_auth_token_policy_id;

    #[test]
    fn test_parametrised_validator() {
        let splash_policy = create_dummy_policy_id(0);
        let farm_auth_policy = create_dummy_policy_id(1);
        let factory_auth_policy = create_dummy_policy_id(2);
        let inflation_box_auth_policy = create_dummy_policy_id(3);
        let zeroth_epoch_start = 100;
        let _ = compute_mint_wp_auth_token_policy_id(
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

        let ff_datum = tx.body.outputs[1].datum().unwrap();
        if let DatumOption::Datum { datum, .. } = ff_datum {
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
}
