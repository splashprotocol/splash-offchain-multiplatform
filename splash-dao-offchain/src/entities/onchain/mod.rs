use cml_chain::transaction::TransactionOutput;
use cml_multi_era::babbage::BabbageTransactionOutput;
use funding_box::{FundingBox, FundingBoxSnapshot};
use inflation_box::{InflationBox, InflationBoxSnapshot};
use make_voting_escrow_order::MakeVotingEscrowOrder;
use permission_manager::{PermManager, PermManagerSnapshot};
use poll_factory::{PollFactory, PollFactorySnapshot};
use smart_farm::{SmartFarm, SmartFarmSnapshot};
use spectrum_cardano_lib::{NetworkId, OutputRef};
use spectrum_offchain::{domain::Has, ledger::TryFromLedger};
use spectrum_offchain_cardano::deployment::DeployedScriptInfo;
use voting_escrow::{VotingEscrow, VotingEscrowSnapshot};
use weighting_poll::{WeightingPoll, WeightingPollSnapshot};

use crate::{
    deployment::ProtocolValidator,
    protocol_config::{
        FarmAuthPolicy, GTAuthPolicy, MintVEIdentifierPolicy, MintWPAuthPolicy, OperatorCreds,
        PermManagerAuthPolicy, SplashPolicy, VEFactoryAuthPolicy,
    },
    routines::inflation::{Slot, TimedOutputRef, WeightingPollEliminated},
    CurrentEpoch, GenesisEpochStartTime,
};

use super::Snapshot;

pub mod farm_factory;
pub mod funding_box;
pub mod inflation_box;
pub mod make_voting_escrow_order;
pub mod permission_manager;
pub mod poll_factory;
pub mod smart_farm;
pub mod voting_escrow;
pub mod voting_escrow_factory;
pub mod weighting_poll;

#[derive(Debug)]
pub enum DaoEntity {
    Inflation(InflationBox),
    PermManager(PermManager),
    WeightingPollFactory(PollFactory),
    SmartFarm(SmartFarm),
    VotingEscrow(VotingEscrow),
    WeightingPoll(WeightingPoll),
    FundingBox(FundingBox),
    MakeVotingEscrowOrder(MakeVotingEscrowOrder),
}

pub type DaoEntitySnapshot = Snapshot<DaoEntity, TimedOutputRef>;

impl<C> TryFromLedger<TransactionOutput, C> for DaoEntitySnapshot
where
    C: Has<SplashPolicy>
        + Has<PermManagerAuthPolicy>
        + Has<MintWPAuthPolicy>
        + Has<FarmAuthPolicy>
        + Has<VEFactoryAuthPolicy>
        + Has<MintVEIdentifierPolicy>
        + Has<GenesisEpochStartTime>
        + Has<GTAuthPolicy>
        + Has<CurrentEpoch>
        + Has<DeployedScriptInfo<{ ProtocolValidator::MintWpAuthPolicy as u8 }>>
        + Has<DeployedScriptInfo<{ ProtocolValidator::VotingEscrow as u8 }>>
        + Has<DeployedScriptInfo<{ ProtocolValidator::Inflation as u8 }>>
        + Has<DeployedScriptInfo<{ ProtocolValidator::PermManager as u8 }>>
        + Has<DeployedScriptInfo<{ ProtocolValidator::WpFactory as u8 }>>
        + Has<DeployedScriptInfo<{ ProtocolValidator::SmartFarm as u8 }>>
        + Has<OperatorCreds>
        + Has<WeightingPollEliminated>
        + Has<NetworkId>
        + Has<TimedOutputRef>
        + Has<OutputRef>,
{
    fn try_from_ledger(repr: &TransactionOutput, ctx: &C) -> Option<Self> {
        if let Some(Snapshot(inflation_box, output_ref)) = InflationBoxSnapshot::try_from_ledger(repr, ctx) {
            Some(Snapshot(DaoEntity::Inflation(inflation_box), output_ref))
        } else if let Some(Snapshot(perm_manager, output_ref)) =
            PermManagerSnapshot::try_from_ledger(repr, ctx)
        {
            Some(Snapshot(DaoEntity::PermManager(perm_manager), output_ref))
        } else if let Some(Snapshot(poll_factory, output_ref)) =
            PollFactorySnapshot::try_from_ledger(repr, ctx)
        {
            Some(Snapshot(
                DaoEntity::WeightingPollFactory(poll_factory),
                output_ref,
            ))
        } else if let Some(Snapshot(smart_farm, output_ref)) = SmartFarmSnapshot::try_from_ledger(repr, ctx) {
            Some(Snapshot(DaoEntity::SmartFarm(smart_farm), output_ref))
        } else if let Some(Snapshot(voting_escrow, output_ref)) =
            VotingEscrowSnapshot::try_from_ledger(repr, ctx)
        {
            Some(Snapshot(DaoEntity::VotingEscrow(voting_escrow), output_ref))
        } else if let Some(Snapshot(weighting_poll, output_ref)) =
            WeightingPollSnapshot::try_from_ledger(repr, ctx)
        {
            Some(Snapshot(DaoEntity::WeightingPoll(weighting_poll), output_ref))
        } else if let Some(Snapshot(funding_box, _output_ref)) =
            FundingBoxSnapshot::try_from_ledger(repr, ctx)
        {
            let timed_output_ref = ctx.select::<TimedOutputRef>();
            Some(Snapshot(DaoEntity::FundingBox(funding_box), timed_output_ref))
        } else {
            None
        }
    }
}
