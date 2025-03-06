use cml_chain::builders::input_builder::{InputBuilderResult, SingleInputBuilder};
use cml_chain::builders::output_builder::SingleOutputBuilderResult;
use cml_chain::builders::redeemer_builder::RedeemerWitnessKey;
use cml_chain::builders::tx_builder::{ChangeSelectionAlgo, SignedTxBuilder, TransactionUnspentOutput};
use cml_chain::builders::witness_builder::{PartialPlutusWitness, PlutusScriptWitness};
use cml_chain::plutus::RedeemerTag;
use cml_chain::transaction::{TransactionInput, TransactionOutput};
use cml_chain::RequiredSigners;
use spectrum_offchain::domain::event::{Predicted, Traced};
use spectrum_offchain_cardano::deployment::DeployedScriptInfo;

use bloom_offchain::execution_engine::bundled::Bundled;
use spectrum_cardano_lib::collateral::Collateral;
use spectrum_cardano_lib::hash::hash_transaction_canonical;
use spectrum_cardano_lib::plutus_data::IntoPlutusData;
use spectrum_cardano_lib::protocol_params::constant_tx_builder;
use spectrum_cardano_lib::transaction::TransactionOutputExtension;
use spectrum_cardano_lib::{AssetClass, AssetName, OutputRef, Token};
use spectrum_offchain::domain::Has;

use crate::constants::fee_deltas::DISTRIBUTE_INFLATION_FEE_DELTA;
use crate::constants::time::DISTRIBUTE_INFLATION_TX_TTL;
use crate::constants::SPLASH_NAME;
use crate::create_change_output::{ChangeOutputCreator, CreateChangeOutput};
use crate::deployment::{DaoScriptData, ProtocolValidator};
use crate::entities::onchain::funding_box::{FundingBox, FundingBoxId};
use crate::entities::onchain::smart_farm::{self};
use crate::entities::onchain::weighting_poll::{self, unsafe_update_wp_state};
use crate::entities::Snapshot;
use crate::protocol_config::{
    EDaoMSigAuthPolicy, FarmAuthPolicy, FarmAuthRefScriptOutput, FarmFactoryAuthPolicy,
    GovProxyRefScriptOutput, InflationAuthPolicy, MintWPAuthPolicy, MintWPAuthRefScriptOutput, OperatorCreds,
    PermManagerAuthPolicy, PermManagerBoxRefScriptOutput, SplashPolicy,
};
use crate::routines::inflation::actions::select_funding_boxes;
use crate::routines::inflation::TimedOutputRef;

use super::{
    AvailableFundingBoxes, CardanoInflationActions, FundingBoxChanges, InflationActions, PermManagerSnapshot,
    Slot, SmartFarmSnapshot, WeightingPollSnapshot,
};

#[async_trait::async_trait]
impl<Ctx> InflationActions<TransactionOutput> for CardanoInflationActions<Ctx>
where
    Ctx: Send
        + Sync
        + Clone
        + Has<Collateral>
        + Has<InflationAuthPolicy>
        + Has<MintWPAuthPolicy>
        + Has<MintWPAuthRefScriptOutput>
        + Has<FarmAuthPolicy>
        + Has<FarmAuthRefScriptOutput>
        + Has<FarmFactoryAuthPolicy>
        + Has<PermManagerBoxRefScriptOutput>
        + Has<GovProxyRefScriptOutput>
        + Has<EDaoMSigAuthPolicy>
        + Has<PermManagerAuthPolicy>
        + Has<OperatorCreds>
        + Has<SplashPolicy>
        + Has<DeployedScriptInfo<{ ProtocolValidator::GovProxy as u8 }>>,
{
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
                        DaoScriptData::global().mint_wp_auth_token.mint_ex_units.clone(),
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
                        DaoScriptData::global().mint_farm_auth_token.ex_units.clone(),
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
        let splash_policy = self.ctx.select::<SplashPolicy>().0;
        let splash_asset_class =
            AssetClass::Token(Token(splash_policy, AssetName::from_utf8(SPLASH_NAME.into())));
        weighting_poll_out.sub_asset(splash_asset_class, splash_emission);

        let mut farm_out = farm_in.clone();
        farm_out.add_asset(splash_asset_class, splash_emission);

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
