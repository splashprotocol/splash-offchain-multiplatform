use std::time::{SystemTime, UNIX_EPOCH};

use cml_chain::transaction::TransactionOutput;

use bloom_offchain::execution_engine::bundled::Bundled;
use cml_chain::PolicyId;
use cml_crypto::RawBytesEncoding;
use pallas_codec::minicbor::Encode;
use pallas_codec::utils::{Int, PlutusBytes};
use spectrum_cardano_lib::transaction::TransactionOutputExtension;
use spectrum_cardano_lib::AssetName;
use spectrum_offchain::data::unique_entity::{Predicted, Traced};
use spectrum_offchain::data::EntitySnapshot;
use spectrum_offchain::ledger::IntoLedger;
use uplc::BigInt;

use crate::assets::SPLASH_AC;
use crate::constants;
use crate::entities::offchain::voting_order::VotingOrder;
use crate::entities::onchain::inflation_box::InflationBox;
use crate::entities::onchain::poll_factory::{unsafe_update_factory_state, PollFactory};
use crate::entities::onchain::smart_farm::SmartFarm;
use crate::entities::onchain::voting_escrow::{unsafe_update_ve_state, VotingEscrow};
use crate::entities::onchain::weighting_poll::{unsafe_update_wp_state, WeightingPoll};

#[async_trait::async_trait]
pub trait InflationActions<Bearer> {
    async fn create_wpoll(
        &self,
        inflation_box: Bundled<InflationBox, Bearer>,
        factory: Bundled<PollFactory, Bearer>,
    ) -> (
        Traced<Predicted<Bundled<InflationBox, Bearer>>>,
        Traced<Predicted<Bundled<PollFactory, Bearer>>>,
        Traced<Predicted<Bundled<WeightingPoll, Bearer>>>,
    );
    async fn eliminate_wpoll(&self, weighting_poll: Bundled<WeightingPoll, Bearer>);
    async fn execute_order(
        &self,
        weighting_poll: Bundled<WeightingPoll, Bearer>,
        order: (VotingOrder, Bundled<VotingEscrow, Bearer>),
    ) -> (
        Traced<Predicted<Bundled<WeightingPoll, Bearer>>>,
        Traced<Predicted<Bundled<VotingEscrow, Bearer>>>,
    );
    async fn distribute_inflation(
        &self,
        weighting_poll: Bundled<WeightingPoll, Bearer>,
        farm: Bundled<SmartFarm, Bearer>,
        farm_weight: u64,
    ) -> (
        Traced<Predicted<Bundled<WeightingPoll, Bearer>>>,
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
        Bundled(inflation_box, inflation_box_in): Bundled<InflationBox, TransactionOutput>,
        Bundled(factory, factory_in): Bundled<PollFactory, TransactionOutput>,
    ) -> (
        Traced<Predicted<Bundled<InflationBox, TransactionOutput>>>,
        Traced<Predicted<Bundled<PollFactory, TransactionOutput>>>,
        Traced<Predicted<Bundled<WeightingPoll, TransactionOutput>>>,
    ) {
        let prev_ib_version = inflation_box.version();
        let (next_inflation_box, rate) = inflation_box.release_next_tranche();
        let mut inflation_box_out = inflation_box_in.clone();
        if let Some(data_mut) = inflation_box_out.data_mut() {
            unsafe_update_factory_state(data_mut, next_inflation_box.last_processed_epoch);
        }
        inflation_box_out.sub_asset(*SPLASH_AC, rate.untag());
        let prev_factory_version = factory.version();
        let (next_factory, fresh_wpoll) = factory.next_weighting_poll();
        let mut factory_out = factory_in.clone();
        if let Some(data_mut) = factory_out.data_mut() {
            unsafe_update_factory_state(data_mut, next_factory.last_poll_epoch);
        }
        let next_traced_ibox = Traced::new(
            Predicted(Bundled(next_inflation_box, inflation_box_out.clone())),
            Some(prev_ib_version),
        );
        let next_traced_factory = Traced::new(
            Predicted(Bundled(next_factory, factory_out.clone())),
            Some(prev_factory_version),
        );
        let wpoll_out = fresh_wpoll.clone().into_ledger(self.ctx);
        let fresh_wpoll = Traced::new(Predicted(Bundled(fresh_wpoll, wpoll_out.clone())), None);
        (next_traced_ibox, next_traced_factory, fresh_wpoll)
    }

    async fn eliminate_wpoll(&self, weighting_poll: Bundled<WeightingPoll, TransactionOutput>) {
        todo!()
    }

    async fn execute_order(
        &self,
        Bundled(weighting_poll, poll_box_in): Bundled<WeightingPoll, TransactionOutput>,
        (order, Bundled(voting_escrow, ve_box_in)): (VotingOrder, Bundled<VotingEscrow, TransactionOutput>),
    ) -> (
        Traced<Predicted<Bundled<WeightingPoll, TransactionOutput>>>,
        Traced<Predicted<Bundled<VotingEscrow, TransactionOutput>>>,
    ) {
        let prev_ve_version = voting_escrow.version();
        let prev_wp_version = weighting_poll.version();

        let mut ve_box_out = ve_box_in.clone();
        if let Some(data_mut) = ve_box_out.data_mut() {
            unsafe_update_ve_state(data_mut, weighting_poll.epoch);
        }

        // Compute the policy for `mint_weighting_power`, to allow us to add the weighting power to WeightingPoll's
        // UTxO.
        let mint_weighting_power_policy = compute_mint_weighting_power_policy_id(
            weighting_poll.epoch,
            order.proposal_auth_policy,
            voting_escrow.gt_policy,
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
            voting_escrow.voting_power(current_posix_time),
        );

        let next_weighting_poll = WeightingPoll {
            distribution: order.distribution,
            ..weighting_poll
        };

        let fresh_wp = Traced::new(
            Predicted(Bundled(next_weighting_poll, poll_box_out)),
            Some(prev_wp_version),
        );

        let next_ve = voting_escrow; // Nothing to change here?
        let fresh_ve = Traced::new(Predicted(Bundled(next_ve, ve_box_out)), Some(prev_ve_version));

        (fresh_wp, fresh_ve)
    }

    async fn distribute_inflation(
        &self,
        weighting_poll: Bundled<WeightingPoll, TransactionOutput>,
        farm: Bundled<SmartFarm, TransactionOutput>,
        farm_weight: u64,
    ) -> (
        Traced<Predicted<Bundled<WeightingPoll, TransactionOutput>>>,
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
    //let params_pd = uplc::PlutusData::Array(vec![
    //    uplc::PlutusData::BigInt(BigInt::Int(Int::from(zeroth_epoch_start as i64))),
    //    uplc::PlutusData::BoundedBytes(PlutusBytes::from(proposal_auth_policy.to_raw_bytes().to_vec())),
    //    uplc::PlutusData::BoundedBytes(PlutusBytes::from(gt_policy.to_raw_bytes().to_vec())),
    //]);
    //let mut params_bytes: Vec<u8> = vec![];
    ////params_pd.encode(&mut params_bytes, &mut ()).unwrap();
    //let bytes = minicbor::to_vec(params_pd).unwrap();

    todo!()
}
