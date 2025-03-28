use cml_chain::address::{BaseAddress, EnterpriseAddress};
use cml_chain::builders::input_builder::{InputBuilderResult, SingleInputBuilder};
use cml_chain::builders::mint_builder::SingleMintBuilder;
use cml_chain::builders::output_builder::TransactionOutputBuilder;
use cml_chain::builders::tx_builder::{ChangeSelectionAlgo, SignedTxBuilder};
use cml_chain::builders::withdrawal_builder::SingleWithdrawalBuilder;
use cml_chain::builders::witness_builder::{PartialPlutusWitness, PlutusScriptWitness};
use cml_chain::certs::{Credential, StakeCredential};
use cml_chain::plutus::{PlutusScript, PlutusV3Script};
use cml_chain::transaction::{DatumOption, TransactionInput, TransactionOutput};
use cml_chain::utils::BigInteger;
use cml_chain::{Deserialize, PolicyId, RequiredSigners, Value};
use cml_crypto::{blake2b256, Ed25519Signature, RawBytesEncoding, TransactionHash};
use log::trace;
use spectrum_cardano_lib::types::TryFromPData;
use spectrum_cardano_lib::value::ValueExtension;
use spectrum_offchain::domain::event::{Predicted, Traced};

use bloom_offchain::execution_engine::bundled::Bundled;
use spectrum_cardano_lib::collateral::Collateral;
use spectrum_cardano_lib::plutus_data::IntoPlutusData;
use spectrum_cardano_lib::protocol_params::COINS_PER_UTXO_BYTE;
use spectrum_cardano_lib::transaction::TransactionOutputExtension;
use spectrum_cardano_lib::{AssetClass, AssetName, NetworkId, OutputRef, Token};
use spectrum_offchain::domain::Has;

use crate::constants::fee_deltas::{
    EXTEND_VOTING_ESCROW_FEE_DELTA, MAKE_VOTING_ESCROW_FEE_DELTA, REDEEM_VOTING_ESCROW_FEE_DELTA,
};
use crate::constants::time::MAX_LOCK_TIME_SECONDS;
use crate::constants::VOTING_ESCROW_TX_TTL;
use crate::create_change_output::{self};
use crate::deployment::DaoScriptData;
use crate::entities::offchain::{
    compute_witness_message, ExtendVotingEscrowOffChainOrder, RedeemVotingEscrowOffChainOrder,
};
use crate::entities::onchain::extend_voting_escrow_order::{
    ExtendVotingEscrowOrderAction, ExtendVotingEscrowOrderBundle,
};
use crate::entities::onchain::make_voting_escrow_order::{
    MakeVotingEscrowOrderAction, MakeVotingEscrowOrderBundle,
};
use crate::entities::onchain::proxy_order_witness::WitnessAction;
use crate::entities::onchain::redeem_voting_escrow::make_redeem_ve_witness_redeemer;
use crate::entities::onchain::voting_escrow::{
    Lock, Owner, VotingEscrow, VotingEscrowAction, VotingEscrowAuthorizedAction, VotingEscrowConfig,
};
use crate::entities::onchain::voting_escrow_factory::{exchange_outputs, FactoryAction, VEFactorySnapshot};
use crate::entities::Snapshot;
use crate::protocol_config::{
    ExtendVotingEscrowOrderRefScriptOutput, ExtendVotingEscrowOrderScriptHash, GTBuiltPolicy,
    MakeVotingEscrowOrderRefScriptOutput, MakeVotingEscrowOrderScriptHash, MintVECompositionPolicy,
    MintVECompositionRefScriptOutput, MintVEIdentifierPolicy, MintVEIdentifierRefScriptOutput, OperatorCreds,
    VEFactoryAuthPolicy, VEFactoryRefScriptOutput, VEFactoryScriptHash, VotingEscrowRefScriptOutput,
    VotingEscrowScriptHash,
};
use crate::routines::actions::{
    compute_identifier_token_asset_name, script_address, BlueprintEstimates, DaoTxBlueprint, WitnessError,
};
use crate::routines::TimedOutputRef;
use crate::time::NetworkTimeProvider;
use crate::NetworkTimeSource;

use super::{
    CardanoInflationActions, ExtendVotingEscrowError, MakeVotingEscrowError, RedeemVotingEscrowError, Slot,
    VoteEscrowActions, VotingEscrowSnapshot,
};

#[async_trait::async_trait]
impl<Ctx> VoteEscrowActions<TransactionOutput> for CardanoInflationActions<Ctx>
where
    Ctx: Send
        + Sync
        + Clone
        + Has<MintVECompositionPolicy>
        + Has<GTBuiltPolicy>
        + Has<VEFactoryRefScriptOutput>
        + Has<VotingEscrowRefScriptOutput>
        + Has<MintVECompositionRefScriptOutput>
        + Has<MintVEIdentifierRefScriptOutput>
        + Has<MakeVotingEscrowOrderRefScriptOutput>
        + Has<VEFactoryScriptHash>
        + Has<MakeVotingEscrowOrderScriptHash>
        + Has<MintVEIdentifierPolicy>
        + Has<NetworkId>
        + Has<VotingEscrowScriptHash>
        + Has<OperatorCreds>
        + Has<ExtendVotingEscrowOrderScriptHash>
        + Has<ExtendVotingEscrowOrderRefScriptOutput>
        + Has<VEFactoryAuthPolicy>
        + Has<Collateral>,
{
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
                let until_secs = until / 1000;
                if until_secs > now_in_seconds {
                    until_secs - now_in_seconds > MAX_LOCK_TIME_SECONDS
                } else {
                    false
                }
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
        let accepted_assets = &ve_factory.get().accepted_assets;
        let legacy_assets = &ve_factory.get().legacy_accepted_assets;

        let mut next_ve_factory = ve_factory.get().clone();
        for (script_hash, names) in mve_tx_output.value().multiasset.iter() {
            for (name, qty) in names.iter() {
                let token = Token(*script_hash, AssetName::from(name.clone()));
                let accepted_asset = accepted_assets.iter().any(|(tok, _)| *tok == token);
                let legacy_asset = legacy_assets.iter().any(|(tok, _)| *tok == token);
                let ac = AssetClass::from(token);
                ve_factory_out_value.add_unsafe(ac, *qty);
                if accepted_asset {
                    next_ve_factory.add_asset_to_inventory((token, *qty), false);
                } else if legacy_asset {
                    next_ve_factory.add_asset_to_inventory((token, *qty), true);
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

        let reference_inputs = vec![
            self.ctx.select::<VEFactoryRefScriptOutput>().0,
            self.ctx.select::<VotingEscrowRefScriptOutput>().0,
            self.ctx.select::<MintVECompositionRefScriptOutput>().0,
            self.ctx.select::<MintVEIdentifierRefScriptOutput>().0,
            self.ctx.select::<MakeVotingEscrowOrderRefScriptOutput>().0,
        ];

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

        let mve_ex_units = DaoScriptData::global().make_voting_escrow_order.ex_units.clone();
        let ve_factory_ex_units = DaoScriptData::global().ve_factory.ex_units.clone();

        let sorted_inputs = if mve_in_ix == 0 {
            vec![
                (mve_input_builder, mve_ex_units),
                (ve_factory_input_builder, ve_factory_ex_units),
            ]
        } else {
            vec![
                (ve_factory_input_builder, ve_factory_ex_units),
                (mve_input_builder, mve_ex_units),
            ]
        };

        // Mint ve_composition tokens --------------------------------------------------------------

        let mut mints = vec![];

        let mint_ve_composition_token_witness = PartialPlutusWitness::new(
            PlutusScriptWitness::Ref(self.ctx.select::<MintVECompositionPolicy>().0),
            cml_chain::plutus::PlutusData::new_integer(BigInteger::from(ve_factory_in_ix)),
        );

        let mint_ve_identifier_ex_units = DaoScriptData::global().mint_identifier.ex_units.clone();

        for (policy_id, names) in voting_escrow_value.multiasset.iter() {
            for (asset_name, qty) in names.iter() {
                let minted_token = crate::create_change_output::Token {
                    policy_id: *policy_id,
                    asset_name: asset_name.clone(),
                    quantity: *qty,
                };
                let mint_ve_composition_builder_result =
                    SingleMintBuilder::new_single_asset(asset_name.clone(), *qty as i64)
                        .plutus_script(mint_ve_composition_token_witness.clone(), vec![].into());
                mints.push((
                    mint_ve_composition_builder_result,
                    minted_token,
                    true,
                    mint_ve_identifier_ex_units.clone(),
                ));
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
        let minted_token = crate::create_change_output::Token {
            policy_id: mint_identifier_policy,
            asset_name: mint_ve_identifier_name.clone(),
            quantity: 1,
        };

        mints.push((
            mint_ve_identifier_builder_result,
            minted_token,
            true,
            mint_ve_identifier_ex_units,
        ));
        mints.sort_by(|(_, t0, _, _), (_, t1, _, _)| t0.policy_id.cmp(&t1.policy_id));

        let id_token = Token(
            mint_identifier_policy,
            spectrum_cardano_lib::AssetName::from(mint_ve_identifier_name.clone()),
        );
        voting_escrow_value.add_unsafe(AssetClass::from(id_token), 1);

        // Add `ve_factory` output -----------------------------------------------------------------

        let mut outputs = vec![];

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

        outputs.push(ve_factory_output.clone());

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
            version: ve_datum.version,
            last_wp_epoch: ve_datum.last_wp_epoch,
            last_gp_deadline: ve_datum.last_gp_deadline,
            redeemed: false,
        };

        let voting_escrow_datum = DatumOption::new_datum(ve_datum.into_pd());
        voting_escrow_value.coin = mve_coin - 1_010_000;

        let voting_escrow_output = TransactionOutputBuilder::new()
            .with_address(script_address(
                self.ctx.select::<VotingEscrowScriptHash>().0,
                self.ctx.select::<NetworkId>(),
            ))
            .with_data(voting_escrow_datum)
            .next()
            .unwrap()
            .with_value(voting_escrow_value.clone())
            .build()
            .unwrap();

        outputs.push(voting_escrow_output.clone());

        let OperatorCreds(_operator_pkh, operator_addr) = self.ctx.select::<OperatorCreds>();
        let mut blueprint = DaoTxBlueprint {
            reference_inputs,
            sorted_inputs,
            outputs,
            sorted_mints: mints,
            withdrawal: None,
            fee_buffer: MAKE_VOTING_ESCROW_FEE_DELTA,
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

        tx_builder
            .add_collateral(InputBuilderResult::from(self.ctx.select::<Collateral>()))
            .unwrap();
        tx_builder.set_validity_start_interval(current_slot.0);
        tx_builder.set_ttl(current_slot.0 + VOTING_ESCROW_TX_TTL);
        tx_builder.set_fee(estimated_fee);
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
        let next_ve_version = OutputRef::new(tx_hash, 1);
        let fresh_ve = Traced::new(
            Predicted(Bundled(
                Snapshot::new(next_ve, next_ve_version),
                voting_escrow_output.output,
            )),
            None,
        );
        Ok((signed_tx_builder, fresh_ve_factory, fresh_ve))
    }

    async fn extend_voting_escrow(
        &self,
        eve_onchain_order: ExtendVotingEscrowOrderBundle<TransactionOutput>,
        eve_offchain_order: ExtendVotingEscrowOffChainOrder,
        Bundled(voting_escrow, ve_box_in): Bundled<VotingEscrowSnapshot, TransactionOutput>,
        Bundled(ve_factory, ve_factory_in): Bundled<VEFactorySnapshot, TransactionOutput>,
        current_slot: Slot,
    ) -> Result<
        (
            SignedTxBuilder,
            Traced<Predicted<Bundled<VEFactorySnapshot, TransactionOutput>>>,
            Traced<Predicted<Bundled<VotingEscrowSnapshot, TransactionOutput>>>,
        ),
        ExtendVotingEscrowError,
    > {
        enum T {
            Order,
            VE,
            VEFactory,
        }
        let order_out_ref = eve_onchain_order.output_ref.output_ref;
        let ve_out_ref = *voting_escrow.version();
        let ve_factory_out_ref = ve_factory.version().output_ref;

        let eve_ex_units = DaoScriptData::global()
            .extend_voting_escrow_order
            .ex_units
            .clone();
        let ve_ex_units = DaoScriptData::global().voting_escrow.ex_units.clone();
        let ve_factory_ex_units = DaoScriptData::global().ve_factory.ex_units.clone();

        let mut values = [
            (T::Order, order_out_ref, eve_ex_units),
            (T::VE, ve_out_ref, ve_ex_units),
            (T::VEFactory, ve_factory_out_ref, ve_factory_ex_units),
        ];
        values.sort_by(|(_, x, _), (_, y, _)| x.cmp(y));

        let order_input_ix = values.iter().position(|(t, _, _)| matches!(t, T::Order)).unwrap();
        let voting_escrow_input_ix = values.iter().position(|(t, _, _)| matches!(t, T::VE)).unwrap();
        let ve_factory_input_ix = values
            .iter()
            .position(|(t, _, _)| matches!(t, T::VEFactory))
            .unwrap();

        // Verification of off-chain message with input `voting_escrow` ----------------------------
        let mut voting_escrow_out = ve_box_in.clone();
        let data_mut = voting_escrow_out.data_mut().unwrap();
        let VotingEscrowConfig { owner, version, .. } =
            VotingEscrowConfig::try_from_pd(data_mut.clone()).unwrap();

        // Verify that witness is authorized by the owner.
        if let Owner::PubKey(bytes) = owner {
            let pk = cml_crypto::PublicKey::from_raw_bytes(&bytes)
                .map_err(|_| ExtendVotingEscrowError::Other("Can't extrat PublicKey from bytes".into()))?;
            let signature = Ed25519Signature::from_raw_bytes(&eve_offchain_order.proof).map_err(|_| {
                ExtendVotingEscrowError::Other("Can't extract Ed25519Signature from bytes".into())
            })?;
            println!("extend_ve_script hash: {}", eve_offchain_order.witness.to_hex());
            println!(" redeemer: {}", eve_offchain_order.witness_input);
            println!(" version: {}", eve_offchain_order.id.version);
            let message = compute_witness_message(
                eve_offchain_order.witness,
                eve_offchain_order.witness_input.clone(),
                eve_offchain_order.id.version as u64,
            )
            .map_err(|_| ExtendVotingEscrowError::Witness(WitnessError::CannotDecodeRedeemer))?;
            println!("message: {}", hex::encode(&message));
            if !pk.verify(&message, &signature) {
                return Err(ExtendVotingEscrowError::Witness(WitnessError::OwnerAuthFailure));
            }
        }

        if version != eve_offchain_order.id.version as u32 {
            return Err(ExtendVotingEscrowError::Witness(
                WitnessError::VEVersionMismatchWithOffchainOrder {
                    voting_escrow_input_version: version,
                    order_version: eve_offchain_order.id.version as u32,
                },
            ));
        }

        if eve_onchain_order.order.ve_datum.version != version + 1 {
            return Err(ExtendVotingEscrowError::Witness(
                WitnessError::VEVersionMismatchWithOnchainProxy {
                    voting_escrow_output_version: version + 1,
                    proxy_version: eve_onchain_order.order.ve_datum.version,
                },
            ));
        }

        let time_source = NetworkTimeSource;
        let locktime_exceeds_limit = match eve_onchain_order.order.ve_datum.locked_until {
            Lock::Def(until) => {
                let now_in_seconds = time_source.network_time().await;
                let until_secs = until / 1000;
                if until_secs > now_in_seconds {
                    until_secs - now_in_seconds > MAX_LOCK_TIME_SECONDS
                } else {
                    false
                }
            }
            Lock::Indef(duration) => duration.as_secs() > MAX_LOCK_TIME_SECONDS,
        };
        if locktime_exceeds_limit {
            return Err(ExtendVotingEscrowError::LocktimeExceedsLimit);
        }

        // Deposit assets into ve_factory -------------------------------------------

        let ve_factory_in_value = ve_factory_in.value();
        let mut ve_factory_out_value = ve_factory_in_value.clone();
        let eve_value = eve_onchain_order.bearer.value();
        let eve_coin = eve_onchain_order.bearer.value().coin;
        let accepted_assets = ve_factory.get().accepted_assets.clone();

        for (script_hash, names) in eve_value.multiasset.iter() {
            for (name, qty) in names.iter() {
                let token = Token(*script_hash, AssetName::from(name.clone()));
                let accepted_asset = accepted_assets.iter().any(|(tok, _)| *tok == token);
                let ac = AssetClass::from(token);
                if accepted_asset {
                    trace!("extend_voting_escrow: adding {} of token: {:?}", *qty, ac);
                    ve_factory_out_value.add_unsafe(ac, *qty);
                } else {
                    return Err(ExtendVotingEscrowError::NonAcceptedAsset);
                }
            }
        }

        let ve_composition_policy = self.ctx.select::<MintVECompositionPolicy>().0;

        // Note that `voting_escrow_value` below contains just the `ve_composition` tokens
        // associated with the new deposits in the `extend_voting_escrow_order`.
        let (ve_composition_qty, mut voting_escrow_value) = exchange_outputs(
            ve_factory_in_value,
            &ve_factory_out_value,
            accepted_assets.clone(),
            ve_composition_policy,
            false,
        );

        let value_with_new_ve_comp_tokens = voting_escrow_value.clone();

        // Add existing tokens from `voting_escrow` input
        for (script_hash, names) in ve_box_in.value().multiasset.iter() {
            for (name, qty) in names.iter() {
                let token = Token(*script_hash, AssetName::from(name.clone()));
                let ac = AssetClass::from(token);
                voting_escrow_value.add_unsafe(ac, *qty);
            }
        }

        let mut next_ve_factory = ve_factory.get().clone();
        next_ve_factory.gt_tokens_available -= ve_composition_qty;

        // `ve_factory` will loan `ve_composition_qty` GT tokens to the newly created `voting_escrow`.
        let gt_token = self.ctx.select::<GTBuiltPolicy>().0;
        let gt_auth_name = spectrum_cardano_lib::AssetName::from(gt_token.asset_name.clone());
        let gt_ac = AssetClass::from(Token(gt_token.policy_id, gt_auth_name));
        ve_factory_out_value.sub_unsafe(gt_ac, ve_composition_qty);

        let reference_inputs = vec![
            self.ctx.select::<VEFactoryRefScriptOutput>().0,
            self.ctx.select::<VotingEscrowRefScriptOutput>().0,
            self.ctx.select::<MintVECompositionRefScriptOutput>().0,
            self.ctx.select::<ExtendVotingEscrowOrderRefScriptOutput>().0,
        ];

        let authorized_action = VotingEscrowAuthorizedAction {
            action: VotingEscrowAction::AddBudgetOrExtend { ve_out_ix: 1 },
            witness: eve_offchain_order.witness,
            version: eve_offchain_order.id.version as u32,
            signature: eve_offchain_order.proof,
        };
        let voting_escrow_script_hash = self.ctx.select::<VotingEscrowScriptHash>().0;

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

        // `ve_factory` input ----------------------------------------------------------------------
        let ve_factory_datum = if let Some(datum) = ve_factory_in.datum() {
            datum
        } else {
            return Err(ExtendVotingEscrowError::VEFactoryDatumNotPresent);
        };

        let ve_factory_script_hash = self.ctx.select::<VEFactoryScriptHash>().0;
        let ve_factory_redeemer = FactoryAction::ExtendPosition {
            ve_in_ix: voting_escrow_input_ix as u64,
        }
        .into_pd();
        let ve_factory_witness = PartialPlutusWitness::new(
            PlutusScriptWitness::Ref(ve_factory_script_hash),
            ve_factory_redeemer,
        );

        let ve_factory_input_builder =
            SingleInputBuilder::new(TransactionInput::from(ve_factory_out_ref), ve_factory_in)
                .plutus_script_inline_datum(ve_factory_witness, vec![].into())
                .unwrap();

        // `extend_voting_escrow_order` input --------------------------------------------------------
        let eve_script_hash = self.ctx.select::<ExtendVotingEscrowOrderScriptHash>().0;
        let order_action = ExtendVotingEscrowOrderAction::Extend {
            order_input_ix: order_input_ix as u32,
            voting_escrow_input_ix: voting_escrow_input_ix as u32,
            ve_factory_input_ix: ve_factory_input_ix as u32,
        };

        let eve_witness = PartialPlutusWitness::new(
            PlutusScriptWitness::Ref(eve_script_hash),
            order_action.clone().into_pd(),
        );

        let eve_input_builder = SingleInputBuilder::new(
            TransactionInput::from(order_out_ref),
            eve_onchain_order.bearer.clone(),
        )
        .plutus_script_inline_datum(eve_witness, vec![].into())
        .unwrap();

        let sorted_inputs = values
            .into_iter()
            .map(|(t, _, ex_units)| match t {
                T::Order => (eve_input_builder.clone(), ex_units),
                T::VE => (voting_escrow_input.clone(), ex_units),
                T::VEFactory => (ve_factory_input_builder.clone(), ex_units),
            })
            .collect::<Vec<_>>();

        let mut mints = vec![];
        // Mint ve_composition tokens --------------------------------------------------------------
        let mint_ve_composition_token_witness = PartialPlutusWitness::new(
            PlutusScriptWitness::Ref(self.ctx.select::<MintVECompositionPolicy>().0),
            cml_chain::plutus::PlutusData::new_integer(BigInteger::from(ve_factory_input_ix)),
        );

        let mint_ex_units = DaoScriptData::global().mint_ve_composition_token.ex_units.clone();

        for (policy_id, names) in value_with_new_ve_comp_tokens.multiasset.iter() {
            for (asset_name, qty) in names.iter() {
                let mint_ve_composition_builder_result =
                    SingleMintBuilder::new_single_asset(asset_name.clone(), *qty as i64)
                        .plutus_script(mint_ve_composition_token_witness.clone(), vec![].into());
                let token = crate::create_change_output::Token {
                    policy_id: *policy_id,
                    asset_name: asset_name.clone(),
                    quantity: *qty,
                };
                mints.push((
                    mint_ve_composition_builder_result,
                    token,
                    true,
                    mint_ex_units.clone(),
                ));
            }
        }

        mints.sort_by(|(_, t0, _, _), (_, t1, _, _)| t0.policy_id.cmp(&t1.policy_id));

        // NOW it is safe to add GT tokens to voting_escrow
        voting_escrow_value.add_unsafe(gt_ac, ve_composition_qty);

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

        // Add `voting_escrow` output --------------------------------------------------------------
        let ve_datum = eve_onchain_order.order.ve_datum;
        assert_eq!(ve_datum.version, version + 1);
        let mut next_ve = voting_escrow.get().clone();
        next_ve.version = ve_datum.version;

        let voting_escrow_datum = DatumOption::new_datum(ve_datum.into_pd());
        voting_escrow_value.coin = ve_box_in.value().coin + eve_coin - 3_000_000;

        let voting_escrow_output = TransactionOutputBuilder::new()
            .with_address(script_address(
                self.ctx.select::<VotingEscrowScriptHash>().0,
                self.ctx.select::<NetworkId>(),
            ))
            .with_data(voting_escrow_datum)
            .next()
            .unwrap()
            .with_value(voting_escrow_value.clone())
            .build()
            .unwrap();

        let outputs = vec![ve_factory_output.clone(), voting_escrow_output.clone()];

        // Set witness script (needed by voting_escrow) --------------------------------------------
        let withdrawal_address = cml_chain::address::RewardAddress::new(
            self.ctx.select::<NetworkId>().into(),
            Credential::new_script(eve_offchain_order.witness),
        );

        let witness_script = PlutusScript::PlutusV3(PlutusV3Script::new(
            hex::decode(&DaoScriptData::global().proxy_order_witness.script_bytes).unwrap(),
        ));

        assert_eq!(witness_script.hash(), eve_offchain_order.witness);

        let witness_action = WitnessAction {
            proxy_order_input_ix: order_input_ix as u32,
            proxy_order_output_reference: order_out_ref,
            proxy_order_script_hash: eve_script_hash,
            proxy_order_redeemer: order_action.into_pd(),
            proxy_order_datum: eve_onchain_order.order.ve_datum.into_pd(),
            owner_redemption: None,
        }
        .into_pd();
        let order_witness =
            PartialPlutusWitness::new(PlutusScriptWitness::Script(witness_script), witness_action);
        let withdrawal_result = SingleWithdrawalBuilder::new(withdrawal_address, 0)
            .plutus_script(order_witness, RequiredSigners::from(vec![]))
            .unwrap();

        let withdrawal = Some((
            withdrawal_result,
            DaoScriptData::global().proxy_order_witness.ex_units.clone(),
        ));

        let OperatorCreds(_operator_pkh, operator_addr) = self.ctx.select::<OperatorCreds>();
        let mut blueprint = DaoTxBlueprint {
            reference_inputs,
            sorted_inputs,
            outputs,
            sorted_mints: mints,
            withdrawal,
            fee_buffer: EXTEND_VOTING_ESCROW_FEE_DELTA,
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

        tx_builder
            .add_collateral(InputBuilderResult::from(self.ctx.select::<Collateral>()))
            .unwrap();
        tx_builder.set_validity_start_interval(current_slot.0);
        tx_builder.set_ttl(current_slot.0 + VOTING_ESCROW_TX_TTL);
        tx_builder.set_fee(estimated_fee);
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
        let next_ve_version = OutputRef::new(tx_hash, 1);
        let fresh_ve = Traced::new(
            Predicted(Bundled(
                Snapshot::new(next_ve, next_ve_version),
                voting_escrow_output.output,
            )),
            None,
        );
        Ok((signed_tx_builder, fresh_ve_factory, fresh_ve))
    }

    async fn redeem_voting_escrow(
        &self,
        offchain_order: RedeemVotingEscrowOffChainOrder,
        Bundled(voting_escrow, ve_box_in): Bundled<VotingEscrowSnapshot, TransactionOutput>,
        Bundled(ve_factory, ve_factory_in): Bundled<VEFactorySnapshot, TransactionOutput>,
        current_slot: Slot,
    ) -> Result<
        (
            SignedTxBuilder,
            Traced<Predicted<Bundled<VEFactorySnapshot, TransactionOutput>>>,
        ),
        RedeemVotingEscrowError,
    > {
        enum T {
            VE,
            VEFactory,
        }
        let ve_out_ref = *voting_escrow.version();
        let ve_factory_out_ref = ve_factory.version().output_ref;
        let ve_ex_units = DaoScriptData::global().voting_escrow.ex_units.clone();
        let ve_factory_ex_units = DaoScriptData::global().ve_factory.ex_units.clone();

        let mut typed_inputs = [
            (T::VE, ve_out_ref, ve_ex_units),
            (T::VEFactory, ve_factory_out_ref, ve_factory_ex_units),
        ];
        typed_inputs.sort_by(|(_, x, _), (_, y, _)| x.cmp(y));

        let voting_escrow_input_ix = typed_inputs
            .iter()
            .position(|(t, _, _)| matches!(t, T::VE))
            .unwrap() as u64;
        let ve_factory_input_ix = typed_inputs
            .iter()
            .position(|(t, _, _)| matches!(t, T::VEFactory))
            .unwrap() as u32;

        // Verification of off-chain message with input `voting_escrow` ----------------------------
        let mut voting_escrow_out = ve_box_in.clone();
        let data_mut = voting_escrow_out.data_mut().unwrap();
        let VotingEscrowConfig {
            owner,
            version,
            locked_until,
            ..
        } = VotingEscrowConfig::try_from_pd(data_mut.clone()).unwrap();

        let network_id = self.ctx.select::<NetworkId>();
        let ve_owner_addr = if let Owner::PubKey(bytes) = owner {
            let pk = cml_crypto::PublicKey::from_raw_bytes(&bytes)
                .map_err(|_| RedeemVotingEscrowError::Other("Can't extrat PublicKey from bytes".into()))?;
            let signature = Ed25519Signature::from_raw_bytes(&offchain_order.proof).map_err(|_| {
                RedeemVotingEscrowError::Other("Can't extract Ed25519Signature from bytes".into())
            })?;
            println!("redeem_ve_script hash: {}", offchain_order.witness.to_hex());
            println!(" redeemer: {}", offchain_order.witness_input);
            println!(" version: {}", offchain_order.id.version);
            let message = compute_witness_message(
                offchain_order.witness,
                offchain_order.witness_input.clone(),
                offchain_order.id.version as u64,
            )
            .map_err(|_| RedeemVotingEscrowError::Witness(WitnessError::CannotDecodeRedeemer))?;
            println!("message: {}", hex::encode(&message));
            if !pk.verify(&message, &signature) {
                return Err(RedeemVotingEscrowError::Witness(WitnessError::OwnerAuthFailure));
            }

            let payment_cred = StakeCredential::new_pub_key(pk.hash());
            if let Some(ref stake_cred) = offchain_order.stake_credential {
                BaseAddress::new(network_id.into(), payment_cred, stake_cred.clone()).to_address()
            } else {
                EnterpriseAddress::new(network_id.into(), payment_cred).to_address()
            }
        } else {
            todo!("Script addresses not yet supported");
        };

        let order_version = offchain_order.id.version as u32;
        if version != order_version {
            return Err(RedeemVotingEscrowError::Witness(
                WitnessError::VEVersionMismatchWithOffchainOrder {
                    voting_escrow_input_version: version,
                    order_version,
                },
            ));
        }

        let time_source = NetworkTimeSource;
        let still_locked = if let Lock::Def(until) = locked_until {
            let now_millis = time_source.network_time().await * 1000;
            now_millis <= until
        } else {
            true
        };
        if still_locked {
            return Err(RedeemVotingEscrowError::VEStillLocked);
        }

        // Return `ve_composition` and GT tokens to VE factory and all deposits to owner -----------

        let mut next_ve_factory = ve_factory.get().clone();
        let mut ve_factory_out_value = ve_factory_in.value().clone();
        let ve_composition_policy = self.ctx.select::<MintVECompositionPolicy>().0;
        let gt_token = self.ctx.select::<GTBuiltPolicy>().0;
        let gt_auth_name = spectrum_cardano_lib::AssetName::from(gt_token.asset_name.clone());
        let gt_ac = AssetClass::from(Token(gt_token.policy_id, gt_auth_name));

        let mint_ve_identifier_policy_id = self.ctx.select::<MintVEIdentifierPolicy>().0;
        let mut mint_ve_identifier_token = None;

        let mut owner_value = Value::zero();
        let mut mints = vec![];
        for ((token @ Token(policy_id, token_name), _), is_legacy_asset) in ve_factory
            .get()
            .accepted_assets
            .iter()
            .map(|t| (t, false))
            .chain(ve_factory.get().legacy_accepted_assets.iter().map(|t| (t, true)))
        {
            let token_name = cml_chain::assets::AssetName::from(*token_name);
            let mut bytes = policy_id.to_raw_bytes().to_vec();
            bytes.extend(token_name.to_raw_bytes());
            let ve_composition_tn = AssetName::try_from(blake2b256(&bytes).to_vec()).unwrap();
            let ve_comp_name_cml = cml_chain::assets::AssetName::from(ve_composition_tn);
            // let ve_comp_token = AssetClass::from(Token(ve_composition_policy, ve_composition_tn));

            for (script_hash, names) in ve_box_in.value().multiasset.iter() {
                if *script_hash == ve_composition_policy {
                    for (name, qty) in names.iter() {
                        if *name == ve_comp_name_cml {
                            owner_value.add_unsafe(AssetClass::from(*token), *qty);
                            // ve_factory_out_value.add_unsafe(ve_comp_token, *qty);
                            ve_factory_out_value.sub_unsafe(AssetClass::from(*token), *qty);
                            next_ve_factory.remove_asset_from_inventory((*token, *qty), is_legacy_asset);

                            // Need to also burn the tokens
                            let mint_ve_composition_token_witness = PartialPlutusWitness::new(
                                PlutusScriptWitness::Ref(ve_composition_policy),
                                cml_chain::plutus::PlutusData::new_integer(BigInteger::from(
                                    ve_factory_input_ix,
                                )),
                            );
                            let mint_ve_composition_builder_result =
                                SingleMintBuilder::new_single_asset(ve_comp_name_cml.clone(), -(*qty as i64))
                                    .plutus_script(
                                        mint_ve_composition_token_witness,
                                        RequiredSigners::from(vec![]),
                                    );
                            let ex_units = DaoScriptData::global().mint_ve_composition_token.ex_units.clone();
                            mints.push((
                                mint_ve_composition_builder_result,
                                create_change_output::Token {
                                    policy_id: ve_composition_policy,
                                    asset_name: ve_comp_name_cml.clone(),
                                    quantity: *qty,
                                },
                                false,
                                ex_units,
                            ));
                        }
                    }
                } else if *script_hash == gt_token.policy_id {
                    assert_eq!(names.len(), 1);
                    let (gt_name_in_ve, qty) = names.front().unwrap();
                    assert_eq!(gt_token.asset_name, *gt_name_in_ve);
                    ve_factory_out_value.add_unsafe(gt_ac, *qty);
                    next_ve_factory.gt_tokens_available += *qty;
                } else if *script_hash == mint_ve_identifier_policy_id {
                    assert_eq!(names.len(), 1);
                    let (ve_ident_name, qty) = names.front().unwrap();
                    assert_eq!(*qty, 1);
                    let burned_token = crate::create_change_output::Token {
                        policy_id: mint_ve_identifier_policy_id,
                        asset_name: ve_ident_name.clone(),
                        quantity: 1, // Even though this is a burn, this must be positive.
                    };
                    mint_ve_identifier_token = Some(burned_token);
                }
            }
        }
        let reference_inputs = vec![
            self.ctx.select::<VEFactoryRefScriptOutput>().0,
            self.ctx.select::<VotingEscrowRefScriptOutput>().0,
            self.ctx.select::<MintVEIdentifierRefScriptOutput>().0,
            self.ctx.select::<MintVECompositionRefScriptOutput>().0,
        ];

        // `voting_escrow` input -------------------------------------------------------------------
        let authorized_action = VotingEscrowAuthorizedAction {
            action: VotingEscrowAction::Redeem {
                ve_factory_in_ix: ve_factory_input_ix,
            },
            witness: offchain_order.witness,
            version: offchain_order.id.version as u32,
            signature: offchain_order.proof,
        };

        let voting_escrow_script_hash = self.ctx.select::<VotingEscrowScriptHash>().0;

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

        // `ve_factory` input ----------------------------------------------------------------------
        let ve_factory_datum = if let Some(datum) = ve_factory_in.datum() {
            datum
        } else {
            return Err(RedeemVotingEscrowError::VEFactoryDatumNotPresent);
        };

        let ve_factory_script_hash = self.ctx.select::<VEFactoryScriptHash>().0;
        let ve_factory_redeemer = FactoryAction::RedeemFromVE {
            ve_in_ix: voting_escrow_input_ix,
        }
        .into_pd();
        let ve_factory_witness = PartialPlutusWitness::new(
            PlutusScriptWitness::Ref(ve_factory_script_hash),
            ve_factory_redeemer,
        );

        let ve_factory_input_builder =
            SingleInputBuilder::new(TransactionInput::from(ve_factory_out_ref), ve_factory_in)
                .plutus_script_inline_datum(ve_factory_witness, vec![].into())
                .unwrap();

        let sorted_inputs = typed_inputs
            .into_iter()
            .map(|(t, _, ex_units)| match t {
                T::VE => (voting_escrow_input.clone(), ex_units),
                T::VEFactory => (ve_factory_input_builder.clone(), ex_units),
            })
            .collect::<Vec<_>>();

        // Burn VE's identifier NFT ------------------------------------------------------------
        // Dummy value
        let ve_factory_output_ref = ve_factory.version().output_ref;

        let mint_ve_identifier_token_witness = PartialPlutusWitness::new(
            PlutusScriptWitness::Ref(mint_ve_identifier_policy_id),
            ve_factory_output_ref.into_pd(),
        );
        let ve_identifier_token = mint_ve_identifier_token.unwrap();
        let mint_ve_identifier_builder_result =
            SingleMintBuilder::new_single_asset(ve_identifier_token.asset_name.clone(), -1)
                .plutus_script(mint_ve_identifier_token_witness, RequiredSigners::from(vec![]));

        let mint_ex_units = DaoScriptData::global().mint_identifier.ex_units.clone();
        mints.push((
            mint_ve_identifier_builder_result,
            ve_identifier_token.clone(),
            false,
            mint_ex_units,
        ));
        mints.sort_by(|(_, t0, _, _), (_, t1, _, _)| t0.policy_id.cmp(&t1.policy_id));

        // Add `ve_factory` output -----------------------------------------------------------------
        let ve_factory_output = TransactionOutputBuilder::new()
            .with_address(script_address(ve_factory_script_hash, network_id))
            .with_data(ve_factory_datum)
            .next()
            .unwrap()
            .with_asset_and_min_required_coin(ve_factory_out_value.multiasset, COINS_PER_UTXO_BYTE)
            .unwrap()
            .build()
            .unwrap();

        // Add `owner` output --------------------------------------------------------------
        let owner_output = TransactionOutputBuilder::new()
            .with_address(ve_owner_addr)
            .next()
            .unwrap()
            .with_asset_and_min_required_coin(owner_value.multiasset, COINS_PER_UTXO_BYTE)
            .unwrap()
            .build()
            .unwrap();

        let outputs = vec![ve_factory_output.clone(), owner_output];

        // Set witness script (needed by voting_escrow) --------------------------------------------
        let withdrawal_address = cml_chain::address::RewardAddress::new(
            self.ctx.select::<NetworkId>().into(),
            Credential::new_script(offchain_order.witness),
        );

        let witness_script = PlutusScript::PlutusV3(PlutusV3Script::new(
            hex::decode(&DaoScriptData::global().redeem_voting_escrow_witness.script_bytes).unwrap(),
        ));

        let ve_factory_bp = self.ctx.select::<VEFactoryAuthPolicy>().0;
        let ve_factory_auth_policy = ve_factory_bp.policy_id;
        let ve_factory_auth_name = spectrum_cardano_lib::AssetName::from(ve_factory_bp.asset_name);
        let ve_identifier_name = spectrum_cardano_lib::AssetName::from(ve_identifier_token.asset_name);

        let witness_redeemer = make_redeem_ve_witness_redeemer(
            offchain_order.stake_credential,
            voting_escrow_input_ix as u32,
            ve_factory_input_ix,
            Token(ve_identifier_token.policy_id, ve_identifier_name),
            Token(ve_factory_auth_policy, ve_factory_auth_name),
            //    SPLASH_AC.into_token().unwrap().0,
            PolicyId::from_hex("7876492e3b82a31b1ce97a8f454cec653a0f6be5c09b90e62d24c152").unwrap(),
            ve_composition_policy,
        );
        let order_witness =
            PartialPlutusWitness::new(PlutusScriptWitness::Script(witness_script), witness_redeemer);
        let withdrawal_result = SingleWithdrawalBuilder::new(withdrawal_address, 0)
            .plutus_script(order_witness, RequiredSigners::from(vec![]))
            .unwrap();

        let withdrawal = Some((
            withdrawal_result,
            DaoScriptData::global()
                .redeem_voting_escrow_witness
                .ex_units
                .clone(),
        ));

        let OperatorCreds(_operator_pkh, operator_addr) = self.ctx.select::<OperatorCreds>();
        let mut blueprint = DaoTxBlueprint {
            reference_inputs,
            sorted_inputs,
            outputs,
            sorted_mints: mints,
            withdrawal,
            fee_buffer: REDEEM_VOTING_ESCROW_FEE_DELTA,
            operator_address: operator_addr.clone(),
        };

        let BlueprintEstimates {
            estimated_fee,
            change_output,
            ..
        } = blueprint.compute_estimated_fee_and_change_output();
        dbg!(change_output.output.value());
        assert!(!change_output.output.value().has_multiassets());
        let change_amount = change_output.output.amount().coin;

        // All ADA in the change-output will be placed into owner's UTxO
        let mut owner_value = blueprint.outputs[1].output.amount().clone();
        owner_value.coin += change_amount;
        blueprint.outputs[1].output.set_amount(owner_value);

        let mut tx_builder = blueprint.build(estimated_fee, None);

        tx_builder
            .add_collateral(InputBuilderResult::from(self.ctx.select::<Collateral>()))
            .unwrap();
        tx_builder.set_validity_start_interval(current_slot.0);
        tx_builder.set_ttl(current_slot.0 + VOTING_ESCROW_TX_TTL);
        tx_builder.set_fee(estimated_fee);
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
        Ok((signed_tx_builder, fresh_ve_factory))
    }
}
