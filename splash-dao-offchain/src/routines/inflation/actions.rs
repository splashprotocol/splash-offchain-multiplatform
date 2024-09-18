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
use cml_chain::{OrderedHashMap, PolicyId, RequiredSigners};
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
use spectrum_offchain::ledger::IntoLedger;
use uplc::PlutusData;
use uplc_pallas_primitives::Fragment;

use crate::assets::SPLASH_AC;
use crate::constants::{self};
use crate::deployment::ProtocolValidator;
use crate::entities::offchain::voting_order::VotingOrder;
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
    InflationBoxSnapshot, PermManagerSnapshot, PollFactorySnapshot, SmartFarmSnapshot, VotingEscrowSnapshot,
    WeightingPollSnapshot,
};

#[async_trait::async_trait]
pub trait InflationActions<Bearer> {
    async fn create_wpoll(
        &self,
        inflation_box: Bundled<InflationBoxSnapshot, Bearer>,
        factory: Bundled<PollFactorySnapshot, Bearer>,
    ) -> (
        SignedTxBuilder,
        Traced<Predicted<Bundled<InflationBoxSnapshot, Bearer>>>,
        Traced<Predicted<Bundled<PollFactorySnapshot, Bearer>>>,
        Traced<Predicted<Bundled<WeightingPollSnapshot, Bearer>>>,
    );
    async fn eliminate_wpoll(
        &self,
        weighting_poll: Bundled<WeightingPollSnapshot, Bearer>,
    ) -> SignedTxBuilder;
    async fn execute_order(
        &self,
        weighting_poll: Bundled<WeightingPollSnapshot, Bearer>,
        order: (VotingOrder, Bundled<VotingEscrowSnapshot, Bearer>),
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
    ) -> (
        SignedTxBuilder,
        Traced<Predicted<Bundled<InflationBoxSnapshot, TransactionOutput>>>,
        Traced<Predicted<Bundled<PollFactorySnapshot, TransactionOutput>>>,
        Traced<Predicted<Bundled<WeightingPollSnapshot, TransactionOutput>>>,
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

        let inflation_tx_input = inflation_input.input.clone();

        tx_builder.add_reference_input(self.ctx.select::<InflationBoxRefScriptOutput>().0.clone());

        let prev_ib_version = *inflation_box.version();
        let (next_inflation_box, emission_rate) = inflation_box.get().release_next_tranche();
        println!(
            "inflation::release_next_tranche --> emission_rate: {}",
            emission_rate.untag()
        );
        let mut inflation_box_out = inflation_box_in.clone();
        let mut amount = inflation_box_out.amount().clone();

        // HACK
        amount.coin -= 2009683;
        inflation_box_out.set_amount(amount);
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

        let wp_factory_tx_input = wp_factory_input.input.clone();

        tx_builder.add_reference_input(self.ctx.select::<PollFactoryRefScriptOutput>().0.clone());

        let prev_factory_version = *factory.version();
        let (next_factory, fresh_wpoll) = factory.unwrap().next_weighting_poll(emission_rate);
        let mut factory_out = factory_in;
        if let Some(data_mut) = factory_out.data_mut() {
            println!("INPUT WP_FACTORY DATUM: {:?}", data_mut);
            unsafe_update_factory_state(data_mut, next_factory.last_poll_epoch.unwrap());
        }

        let mint_action = if inflation_tx_input < wp_factory_tx_input {
            tx_builder.add_input(inflation_input).unwrap();
            tx_builder.add_input(wp_factory_input).unwrap();
            tx_builder.set_exunits(
                RedeemerWitnessKey::new(RedeemerTag::Spend, 0),
                INFLATION_BOX_EX_UNITS,
            );
            tx_builder.set_exunits(
                RedeemerWitnessKey::new(RedeemerTag::Spend, 1),
                WP_FACTORY_EX_UNITS,
            );
            MintAction::MintAuthToken {
                factory_in_ix: 1,
                inflation_box_in_ix: 0,
            }
        } else {
            tx_builder.add_input(wp_factory_input).unwrap();
            tx_builder.add_input(inflation_input).unwrap();
            tx_builder.set_exunits(
                RedeemerWitnessKey::new(RedeemerTag::Spend, 0),
                WP_FACTORY_EX_UNITS,
            );
            tx_builder.set_exunits(
                RedeemerWitnessKey::new(RedeemerTag::Spend, 1),
                INFLATION_BOX_EX_UNITS,
            );
            MintAction::MintAuthToken {
                factory_in_ix: 0,
                inflation_box_in_ix: 1,
            }
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
        let reward_address = self.ctx.select::<Reward>().0.clone();
        //let gov_witness_script_hash = self
        //    .ctx
        //    .select::<DeployedScriptInfo<{ ProtocolValidator::GovProxy as u8 }>>()
        //    .script_hash;
        //let gp_witness = PartialPlutusWitness::new(
        //    PlutusScriptWitness::Ref(gov_witness_script_hash),
        //    cml_chain::plutus::PlutusData::new_list(vec![]), // dummy value (this validator doesn't require redeemer)
        //);
        //let withdrawal_result = SingleWithdrawalBuilder::new(reward_address.clone(), 0)
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

        let execution_fee_address: Address = reward_address.into();

        // Build tx, change is execution fee.
        let signed_tx_builder = tx_builder
            .build(ChangeSelectionAlgo::Default, &execution_fee_address)
            .unwrap();
        let tx_body = signed_tx_builder.body();

        let tx_hash = hash_transaction_canonical(&tx_body);

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
        )
    }

    async fn eliminate_wpoll(
        &self,
        Bundled(weighting_poll, weighting_poll_in): Bundled<WeightingPollSnapshot, TransactionOutput>,
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

#[cfg(test)]
mod tests {
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
