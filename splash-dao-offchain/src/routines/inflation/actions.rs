use std::time::{SystemTime, UNIX_EPOCH};

use cml_chain::address::Address;
use cml_chain::assets::AssetBundle;
use cml_chain::builders::input_builder::SingleInputBuilder;
use cml_chain::builders::mint_builder::SingleMintBuilder;
use cml_chain::builders::output_builder::SingleOutputBuilderResult;
use cml_chain::builders::redeemer_builder::RedeemerWitnessKey;
use cml_chain::builders::tx_builder::ChangeSelectionAlgo;
use cml_chain::builders::withdrawal_builder::SingleWithdrawalBuilder;
use cml_chain::builders::witness_builder::{PartialPlutusWitness, PlutusScriptWitness};
use cml_chain::plutus::{PlutusV2Script, RedeemerTag};
use cml_chain::transaction::{ScriptRef, TransactionInput, TransactionOutput};

use bloom_offchain::execution_engine::bundled::Bundled;
use cml_chain::{OrderedHashMap, PolicyId, Script};
use cml_crypto::{blake2b256, RawBytesEncoding, ScriptHash};
use spectrum_cardano_lib::hash::hash_transaction_canonical;
use spectrum_cardano_lib::plutus_data::IntoPlutusData;
use spectrum_cardano_lib::protocol_params::constant_tx_builder;
use spectrum_cardano_lib::transaction::TransactionOutputExtension;
use spectrum_cardano_lib::{AssetName, OutputRef};
use spectrum_offchain::data::unique_entity::{Predicted, Traced};
use spectrum_offchain::data::{EntitySnapshot, Stable};
use spectrum_offchain::ledger::IntoLedger;
use spectrum_offchain_cardano::creds::operator_creds;
use uplc::tx::apply_params_to_script;
use uplc::{plutus_data_to_bytes, BigInt};
use uplc_pallas_codec::utils::{Bytes, Int, PlutusBytes};
use uplc_pallas_traverse::ComputeHash;

use crate::assets::SPLASH_AC;
use crate::constants::{
    self, INFLATION_SCRIPT, MINT_WEIGHTING_POWER_SCRIPT, MINT_WP_AUTH_TOKEN_SCRIPT, WP_FACTORY_SCRIPT,
};
use crate::entities::offchain::voting_order::VotingOrder;
use crate::entities::onchain::inflation_box::{InflationBox, INFLATION_BOX_EX_UNITS};
use crate::entities::onchain::poll_factory::{
    unsafe_update_factory_state, FactoryRedeemer, PollFactory, PollFactoryAction, GOV_PROXY_EX_UNITS,
    WP_FACTORY_EX_UNITS,
};
use crate::entities::onchain::smart_farm::SmartFarm;
use crate::entities::onchain::voting_escrow::{unsafe_update_ve_state, VotingEscrow};
use crate::entities::onchain::weighting_poll::{
    unsafe_update_wp_state, MintAction, WeightingPoll, MINT_WP_AUTH_EX_UNITS,
};
use crate::entities::Snapshot;
use crate::protocol_config::{ProtocolConfig, TX_FEE_CORRECTION};

use super::{InflationBoxSnapshot, PollFactorySnapshot, VotingEscrowSnapshot, WeightingPollSnapshot};

#[async_trait::async_trait]
pub trait InflationActions<Bearer> {
    async fn create_wpoll(
        &self,
        config: &ProtocolConfig,
        zeroth_epoch_start: u32,
        inflation_box: Bundled<InflationBoxSnapshot, Bearer>,
        factory: Bundled<PollFactorySnapshot, Bearer>,
    ) -> (
        Traced<Predicted<Bundled<InflationBoxSnapshot, Bearer>>>,
        Traced<Predicted<Bundled<PollFactorySnapshot, Bearer>>>,
        Traced<Predicted<Bundled<WeightingPollSnapshot, Bearer>>>,
    );
    async fn eliminate_wpoll(&self, weighting_poll: Bundled<WeightingPollSnapshot, Bearer>);
    async fn execute_order(
        &self,
        weighting_poll: Bundled<WeightingPollSnapshot, Bearer>,
        order: (VotingOrder, Bundled<VotingEscrowSnapshot, Bearer>),
    ) -> (
        Traced<Predicted<Bundled<WeightingPollSnapshot, Bearer>>>,
        Traced<Predicted<Bundled<VotingEscrowSnapshot, Bearer>>>,
    );
    async fn distribute_inflation(
        &self,
        weighting_poll: Bundled<WeightingPollSnapshot, Bearer>,
        farm: Bundled<SmartFarm, Bearer>,
        farm_weight: u64,
    ) -> (
        Traced<Predicted<Bundled<WeightingPollSnapshot, Bearer>>>,
        Traced<Predicted<Bundled<SmartFarm, Bearer>>>,
    );
}

pub struct CardanoInflationActions<Ctx> {
    ctx: Ctx,
}

#[async_trait::async_trait]
impl<Ctx> InflationActions<TransactionOutput> for CardanoInflationActions<Ctx>
where
    Ctx: Send + Sync + Copy,
{
    async fn create_wpoll(
        &self,
        config: &ProtocolConfig,
        zeroth_epoch_start: u32,
        Bundled(inflation_box, inflation_box_in): Bundled<InflationBoxSnapshot, TransactionOutput>,
        Bundled(factory, factory_in): Bundled<PollFactorySnapshot, TransactionOutput>,
    ) -> (
        Traced<Predicted<Bundled<InflationBoxSnapshot, TransactionOutput>>>,
        Traced<Predicted<Bundled<PollFactorySnapshot, TransactionOutput>>>,
        Traced<Predicted<Bundled<WeightingPollSnapshot, TransactionOutput>>>,
    ) {
        let mut tx_builder = constant_tx_builder();

        let weighting_power_policy = compute_mint_weighting_power_policy_id(
            zeroth_epoch_start,
            config.wpoll_auth_policy,
            config.gt_policy,
        );

        let inflation_script_hash = compute_inflation_box_script_hash(
            config.splash_policy,
            config.wpoll_auth_policy,
            weighting_power_policy,
            zeroth_epoch_start,
        );
        let inflation_script = PartialPlutusWitness::new(
            PlutusScriptWitness::Ref(inflation_script_hash),
            cml_chain::plutus::PlutusData::Integer(cml_chain::utils::BigInt::from(0)),
        );

        let inflation_input = SingleInputBuilder::new(
            TransactionInput::from(*(inflation_box.version())),
            inflation_box_in.clone(),
        )
        .plutus_script_inline_datum(inflation_script, vec![])
        .unwrap();

        let inflation_input_tx_hash = inflation_input.input.transaction_id;

        tx_builder.add_reference_input(config.inflation_box_ref_script.clone());
        tx_builder.add_input(inflation_input).unwrap();

        let prev_ib_version = *inflation_box.version();
        let (next_inflation_box, rate) = inflation_box.get().release_next_tranche();
        let mut inflation_box_out = inflation_box_in.clone();
        if let Some(data_mut) = inflation_box_out.data_mut() {
            unsafe_update_factory_state(data_mut, next_inflation_box.last_processed_epoch);
        }
        inflation_box_out.sub_asset(*SPLASH_AC, rate.untag());
        let inflation_output = SingleOutputBuilderResult::new(inflation_box_out.clone());
        tx_builder.add_output(inflation_output).unwrap();

        // WP factory
        let wp_factory_script_hash = compute_wp_factory_script_hash(
            config.wpoll_auth_policy,
            factory.get().stable_id.gov_witness_script_hash,
        );

        let factory_redeemer = FactoryRedeemer {
            successor_ix: 1,
            action: PollFactoryAction::CreatePoll,
        };
        let wp_factory_script = PartialPlutusWitness::new(
            PlutusScriptWitness::Ref(wp_factory_script_hash),
            factory_redeemer.into_pd(),
        );

        let wp_factory_input =
            SingleInputBuilder::new(TransactionInput::from(*(factory.version())), factory_in.clone())
                .plutus_script_inline_datum(wp_factory_script, vec![])
                .unwrap();

        let wp_factory_input_tx_hash = wp_factory_input.input.transaction_id;

        tx_builder.add_reference_input(config.poll_factory_ref_script.clone());
        tx_builder.add_input(wp_factory_input).unwrap();

        let gov_witness_script_hash = factory.get().stable_id.gov_witness_script_hash;
        let prev_factory_version = *factory.version();
        let (next_factory, fresh_wpoll) = factory.unwrap().next_weighting_poll(config.farm_auth_policy);
        let mut factory_out = factory_in.clone();
        if let Some(data_mut) = factory_out.data_mut() {
            unsafe_update_factory_state(data_mut, next_factory.last_poll_epoch);
        }

        let mint_action = if inflation_input_tx_hash < wp_factory_input_tx_hash {
            tx_builder.set_exunits(
                RedeemerWitnessKey::new(RedeemerTag::Spend, 0),
                INFLATION_BOX_EX_UNITS,
            );
            tx_builder.set_exunits(
                RedeemerWitnessKey::new(RedeemerTag::Spend, 1),
                WP_FACTORY_EX_UNITS,
            );
            MintAction::MintAuthToken { factory_in_ix: 1 }
        } else {
            tx_builder.set_exunits(
                RedeemerWitnessKey::new(RedeemerTag::Spend, 0),
                WP_FACTORY_EX_UNITS,
            );
            tx_builder.set_exunits(
                RedeemerWitnessKey::new(RedeemerTag::Spend, 1),
                INFLATION_BOX_EX_UNITS,
            );
            MintAction::MintAuthToken { factory_in_ix: 0 }
        };

        // Mint wp_auth token
        let mint_wp_auth_token_script_hash = compute_mint_wp_auth_token_policy_id(
            config.splash_policy,
            config.farm_auth_policy,
            config.factory_auth_policy,
            zeroth_epoch_start,
        );
        let mint_wp_auth_token_witness = PartialPlutusWitness::new(
            PlutusScriptWitness::Ref(mint_wp_auth_token_script_hash),
            mint_action.into_pd(),
        );
        let (_operator_sk, operator_pkh, _operator_addr) =
            operator_creds(&config.operator_sk, config.node_magic);

        let mut buffer = [0u8; 128];
        minicbor::encode(inflation_box.get().last_processed_epoch, buffer.as_mut()).unwrap();
        let token_name = blake2b256(buffer.as_ref());
        let asset = cml_chain::assets::AssetName::new(token_name.to_vec()).unwrap();
        let minting_policy = SingleMintBuilder::new_single_asset(asset, 1)
            .plutus_script(mint_wp_auth_token_witness, vec![operator_pkh]);
        tx_builder.add_mint(minting_policy).unwrap();
        tx_builder.set_exunits(
            RedeemerWitnessKey::new(RedeemerTag::Mint, 0),
            MINT_WP_AUTH_EX_UNITS,
        );

        // Contracts require that weighting_poll output resides at index 1.
        let mut wpoll_out = fresh_wpoll.clone().into_ledger(self.ctx);
        // Add wp_auth_token to this output.
        //match &mut wpoll_out {
        //    TransactionOutput::AlonzoFormatTxOut(tx_out) => {
        //        //fn from(bundle: OrderedHashMap<PolicyId, OrderedHashMap<AssetName, T>>) -> Self {
        //        //let asset_pair =OrderedHashMap::from_iter(vec![(token_name, 1)]);
        //        //let hash_map = OrderedHashMap::from_iter(())
        //        //            tx_out.amount.multiasset.checked_add(AssetBundle)
        //    }
        //    TransactionOutput::ConwayFormatTxOut(tx_out) => tx_out.amount.clone(),
        //}
        let weighting_poll_output = SingleOutputBuilderResult::new(wpoll_out.clone());
        tx_builder.add_output(weighting_poll_output).unwrap();

        let factory_output = SingleOutputBuilderResult::new(factory_out.clone());
        tx_builder.add_output(factory_output).unwrap();

        // Set Governance Proxy witness script
        let reward_address = config.reward_address.clone();
        let gp_witness = PartialPlutusWitness::new(
            PlutusScriptWitness::Ref(gov_witness_script_hash),
            cml_chain::plutus::PlutusData::new_list(vec![]), // dummy value (this validator doesn't require redeemer)
        );
        let withdrawal_result = SingleWithdrawalBuilder::new(reward_address.clone(), 0)
            .plutus_script(gp_witness, vec![])
            .unwrap();
        tx_builder.add_withdrawal(withdrawal_result);
        tx_builder.set_exunits(
            RedeemerWitnessKey::new(RedeemerTag::Reward, 0),
            GOV_PROXY_EX_UNITS,
        );

        tx_builder.add_collateral(config.collateral.0.clone()).unwrap();

        let estimated_tx_fee = tx_builder.min_fee(true).unwrap();
        tx_builder.set_fee(estimated_tx_fee + TX_FEE_CORRECTION);

        let execution_fee_address: Address = reward_address.into();
        // Build tx, change is execution fee.
        let tx_body = tx_builder
            .clone()
            .build(ChangeSelectionAlgo::Default, &execution_fee_address)
            .unwrap()
            .body();

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
        (next_traced_ibox, next_traced_factory, fresh_wpoll)
    }

    async fn eliminate_wpoll(&self, weighting_poll: Bundled<WeightingPollSnapshot, TransactionOutput>) {
        todo!()
    }

    async fn execute_order(
        &self,
        Bundled(weighting_poll, poll_box_in): Bundled<WeightingPollSnapshot, TransactionOutput>,
        (order, Bundled(voting_escrow, ve_box_in)): (
            VotingOrder,
            Bundled<VotingEscrowSnapshot, TransactionOutput>,
        ),
    ) -> (
        Traced<Predicted<Bundled<WeightingPollSnapshot, TransactionOutput>>>,
        Traced<Predicted<Bundled<VotingEscrowSnapshot, TransactionOutput>>>,
    ) {
        let prev_ve_version = voting_escrow.version();
        let prev_wp_version = weighting_poll.version();

        let mut ve_box_out = ve_box_in.clone();
        if let Some(data_mut) = ve_box_out.data_mut() {
            unsafe_update_ve_state(data_mut, weighting_poll.get().epoch);
        }

        // Compute the policy for `mint_weighting_power`, to allow us to add the weighting power to WeightingPoll's
        // UTxO.
        let mint_weighting_power_policy = compute_mint_weighting_power_policy_id(
            weighting_poll.get().epoch,
            order.proposal_auth_policy,
            voting_escrow.get().gt_policy,
        );
        let current_posix_time = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs();
        let mut poll_box_out = poll_box_in.clone();
        if let Some(data_mut) = poll_box_out.data_mut() {
            unsafe_update_wp_state(data_mut, &order.distribution);
        }
        poll_box_out.add_asset(
            spectrum_cardano_lib::AssetClass::Token((
                mint_weighting_power_policy,
                AssetName::try_from(vec![constants::GT_NAME]).unwrap(),
            )),
            voting_escrow.get().voting_power(current_posix_time),
        );

        let next_weighting_poll = WeightingPoll {
            distribution: order.distribution,
            ..weighting_poll.get().clone()
        };

        let next_wp_version = *prev_wp_version; // TODO: fix
        let fresh_wp = Traced::new(
            Predicted(Bundled(
                Snapshot::new(next_weighting_poll, next_wp_version),
                poll_box_out,
            )),
            Some(*prev_wp_version),
        );

        let next_ve_version = *prev_ve_version; // TODO: fix
        let next_ve = *voting_escrow.get(); // Nothing to change here?
        let fresh_ve = Traced::new(
            Predicted(Bundled(Snapshot::new(next_ve, next_ve_version), ve_box_out)),
            Some(*prev_ve_version),
        );

        (fresh_wp, fresh_ve)
    }

    async fn distribute_inflation(
        &self,
        weighting_poll: Bundled<WeightingPollSnapshot, TransactionOutput>,
        farm: Bundled<SmartFarm, TransactionOutput>,
        farm_weight: u64,
    ) -> (
        Traced<Predicted<Bundled<WeightingPollSnapshot, TransactionOutput>>>,
        Traced<Predicted<Bundled<SmartFarm, TransactionOutput>>>,
    ) {
        todo!()
    }
}

fn compute_mint_weighting_power_policy_id(
    zeroth_epoch_start: u32,
    proposal_auth_policy: PolicyId,
    gt_policy: PolicyId,
) -> PolicyId {
    let params_pd = uplc::PlutusData::Array(vec![
        uplc::PlutusData::BigInt(BigInt::Int(Int::from(zeroth_epoch_start as i64))),
        uplc::PlutusData::BoundedBytes(PlutusBytes::from(proposal_auth_policy.to_raw_bytes().to_vec())),
        uplc::PlutusData::BoundedBytes(PlutusBytes::from(gt_policy.to_raw_bytes().to_vec())),
    ]);
    let params_bytes = plutus_data_to_bytes(&params_pd).unwrap();
    let script = PlutusV2Script::new(hex::decode(MINT_WEIGHTING_POWER_SCRIPT).unwrap());

    let script_bytes = apply_params_to_script(&params_bytes, script.get()).unwrap();

    let script_hash =
        uplc_pallas_primitives::babbage::PlutusV2Script(Bytes::from(script_bytes)).compute_hash();

    PolicyId::from_raw_bytes(script_hash.as_slice()).unwrap()
}

fn compute_inflation_box_script_hash(
    splash_policy: PolicyId,
    wp_auth_policy: PolicyId,
    weighting_power_policy: PolicyId,
    zeroth_epoch_start: u32,
) -> ScriptHash {
    let params_pd = uplc::PlutusData::Array(vec![
        uplc::PlutusData::BoundedBytes(PlutusBytes::from(splash_policy.to_raw_bytes().to_vec())),
        uplc::PlutusData::BoundedBytes(PlutusBytes::from(wp_auth_policy.to_raw_bytes().to_vec())),
        uplc::PlutusData::BoundedBytes(PlutusBytes::from(weighting_power_policy.to_raw_bytes().to_vec())),
        uplc::PlutusData::BigInt(BigInt::Int(Int::from(zeroth_epoch_start as i64))),
    ]);
    apply_params_validator(params_pd, INFLATION_SCRIPT)
}

fn compute_wp_factory_script_hash(
    wp_auth_policy: PolicyId,
    gov_witness_script_hash: ScriptHash,
) -> ScriptHash {
    let params_pd = uplc::PlutusData::Array(vec![
        uplc::PlutusData::BoundedBytes(PlutusBytes::from(wp_auth_policy.to_raw_bytes().to_vec())),
        uplc::PlutusData::BoundedBytes(PlutusBytes::from(gov_witness_script_hash.to_raw_bytes().to_vec())),
    ]);
    apply_params_validator(params_pd, WP_FACTORY_SCRIPT)
}

fn compute_mint_wp_auth_token_policy_id(
    splash_policy: PolicyId,
    farm_auth_policy: PolicyId,
    factory_auth_policy: PolicyId,
    zeroth_epoch_start: u32,
) -> PolicyId {
    let params_pd = uplc::PlutusData::Array(vec![
        uplc::PlutusData::BoundedBytes(PlutusBytes::from(splash_policy.to_raw_bytes().to_vec())),
        uplc::PlutusData::BoundedBytes(PlutusBytes::from(farm_auth_policy.to_raw_bytes().to_vec())),
        uplc::PlutusData::BoundedBytes(PlutusBytes::from(factory_auth_policy.to_raw_bytes().to_vec())),
        uplc::PlutusData::BigInt(BigInt::Int(Int::from(zeroth_epoch_start as i64))),
    ]);
    apply_params_validator(params_pd, MINT_WP_AUTH_TOKEN_SCRIPT)
}

fn apply_params_validator(params_pd: uplc::PlutusData, script: &str) -> ScriptHash {
    let params_bytes = plutus_data_to_bytes(&params_pd).unwrap();
    let script = PlutusV2Script::new(hex::decode(script).unwrap());

    let script_bytes = apply_params_to_script(&params_bytes, script.get()).unwrap();

    let script_hash =
        uplc_pallas_primitives::babbage::PlutusV2Script(Bytes::from(script_bytes)).compute_hash();

    PolicyId::from_raw_bytes(script_hash.as_slice()).unwrap()
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
        let zeroth_epoch_start = 100;
        let _ = compute_mint_wp_auth_token_policy_id(
            splash_policy,
            farm_auth_policy,
            factory_auth_policy,
            zeroth_epoch_start,
        );
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
