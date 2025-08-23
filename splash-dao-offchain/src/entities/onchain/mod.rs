use cml_chain::{
    auxdata::{Metadata, TransactionMetadatum},
    transaction::TransactionOutput,
};
use cml_core::{serialization::FromBytes, Int};
use cml_crypto::{RawBytesEncoding, ScriptHash};
use extend_voting_escrow_order::{ExtendVotingEscrowOnchainOrder, ExtendVotingEscrowOrderBundle};
use funding_box::{FundingBox, FundingBoxSnapshot};
use inflation_box::{InflationBox, InflationBoxSnapshot};
use make_voting_escrow_order::{MakeVotingEscrowOrder, MakeVotingEscrowOrderBundle};
use permission_manager::{PermManager, PermManagerSnapshot};
use poll_factory::{PollFactory, PollFactorySnapshot};
use redeem_voting_escrow::{RedeemVotingEscrowOnchainOrder, RedeemVotingEscrowOrderBundle};
use serde::{Deserialize, Serialize};
use smart_farm::{SmartFarm, SmartFarmSnapshot};
use spectrum_cardano_lib::{NetworkId, OutputRef};
use spectrum_offchain::{
    backlog::data::{OrderWeight, Weighted},
    domain::{order::UniqueOrder, Has},
    ledger::TryFromLedger,
};
use spectrum_offchain_cardano::{deployment::DeployedScriptInfo, raw_bytes::RawBytes};
use voting_escrow::{Lock, Owner, VotingEscrow, VotingEscrowSnapshot};
use voting_escrow_factory::{VEFactory, VEFactorySnapshot};
use weighting_poll::{WeightingPoll, WeightingPollSnapshot};
use wpoll_vote_order::{WPollVoteOnchainOrder, WPollVoteOrderBundle};

use crate::{
    deployment::ProtocolValidator,
    protocol_config::{
        GTAuthPolicy, InflationAuthPolicy, OperatorCreds, PermManagerAuthPolicy, SplashPolicy,
        VEFactoryAuthPolicy, WPFactoryAuthPolicy,
    },
    routines::TimedOutputRef,
    CurrentEpoch, GenesisEpochStartTime,
};

use super::Snapshot;

pub mod extend_voting_escrow_order;
pub mod farm_factory;
pub mod funding_box;
pub mod inflation_box;
pub mod make_voting_escrow_order;
pub mod permission_manager;
pub mod poll_factory;
pub mod proxy_order_witness;
pub mod redeem_voting_escrow;
pub mod smart_farm;
pub mod voting_escrow;
pub mod voting_escrow_factory;
pub mod weighting_poll;
pub mod wpoll_vote_order;

#[derive(Debug)]
pub enum DaoEntity {
    Inflation(InflationBox),
    PermManager(PermManager),
    WeightingPollFactory(PollFactory),
    SmartFarm(SmartFarm),
    VotingEscrow(VotingEscrow),
    VotingEscrowFactory(VEFactory),
    WeightingPoll(WeightingPoll),
    FundingBox(FundingBox),
    MakeVotingEscrowOrder(MakeVotingEscrowOrder),
    ExtendVotingEscrowOrder(ExtendVotingEscrowOnchainOrder),
    RedeemVotingEscrowOrder(RedeemVotingEscrowOnchainOrder),
    WPollVoteOrder(WPollVoteOnchainOrder),
}

pub type DaoEntitySnapshot = Snapshot<DaoEntity, TimedOutputRef>;

impl<C> TryFromLedger<TransactionOutput, C> for DaoEntitySnapshot
where
    C: Has<SplashPolicy>
        + Has<PermManagerAuthPolicy>
        + Has<VEFactoryAuthPolicy>
        + Has<InflationAuthPolicy>
        + Has<WPFactoryAuthPolicy>
        + Has<GenesisEpochStartTime>
        + Has<GTAuthPolicy>
        + Has<CurrentEpoch>
        + Has<DeployedScriptInfo<{ ProtocolValidator::MintIdentifier as u8 }>>
        + Has<DeployedScriptInfo<{ ProtocolValidator::MintWpAuthPolicy as u8 }>>
        + Has<DeployedScriptInfo<{ ProtocolValidator::VotingEscrow as u8 }>>
        + Has<DeployedScriptInfo<{ ProtocolValidator::Inflation as u8 }>>
        + Has<DeployedScriptInfo<{ ProtocolValidator::PermManager as u8 }>>
        + Has<DeployedScriptInfo<{ ProtocolValidator::WpFactory as u8 }>>
        + Has<DeployedScriptInfo<{ ProtocolValidator::VeFactory as u8 }>>
        + Has<DeployedScriptInfo<{ ProtocolValidator::SmartFarm as u8 }>>
        + Has<DeployedScriptInfo<{ ProtocolValidator::MakeVeOrder as u8 }>>
        + Has<DeployedScriptInfo<{ ProtocolValidator::ExtendVeOrder as u8 }>>
        + Has<DeployedScriptInfo<{ ProtocolValidator::WPollVoteOrder as u8 }>>
        + Has<DeployedScriptInfo<{ ProtocolValidator::RedeemVeOrder as u8 }>>
        + Has<OperatorCreds>
        + Has<Option<Metadata>>
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
        } else if let Some(Snapshot(ve_factory, output_ref)) = VEFactorySnapshot::try_from_ledger(repr, ctx) {
            Some(Snapshot(DaoEntity::VotingEscrowFactory(ve_factory), output_ref))
        } else if let Some(Snapshot(smart_farm, output_ref)) = SmartFarmSnapshot::try_from_ledger(repr, ctx) {
            Some(Snapshot(DaoEntity::SmartFarm(smart_farm), output_ref))
        } else if let Some(Snapshot(voting_escrow, output_ref)) =
            VotingEscrowSnapshot::try_from_ledger(repr, ctx)
        {
            let timed_output_ref = ctx.select::<TimedOutputRef>();
            Some(Snapshot(DaoEntity::VotingEscrow(voting_escrow), timed_output_ref))
        } else if let Some(Snapshot(weighting_poll, output_ref)) =
            WeightingPollSnapshot::try_from_ledger(repr, ctx)
        {
            Some(Snapshot(DaoEntity::WeightingPoll(weighting_poll), output_ref))
        } else if let Some(Snapshot(funding_box, _output_ref)) =
            FundingBoxSnapshot::try_from_ledger(repr, ctx)
        {
            let timed_output_ref = ctx.select::<TimedOutputRef>();
            Some(Snapshot(DaoEntity::FundingBox(funding_box), timed_output_ref))
        } else if let Some(mve_order) = MakeVotingEscrowOrder::try_from_ledger(repr, ctx) {
            let timed_output_ref = ctx.select::<TimedOutputRef>();
            Some(Snapshot(
                DaoEntity::MakeVotingEscrowOrder(mve_order),
                timed_output_ref,
            ))
        } else if let Some(eve_order) = ExtendVotingEscrowOnchainOrder::try_from_ledger(repr, ctx) {
            let timed_output_ref = ctx.select::<TimedOutputRef>();
            Some(Snapshot(
                DaoEntity::ExtendVotingEscrowOrder(eve_order),
                timed_output_ref,
            ))
        } else if let Some(order) = WPollVoteOnchainOrder::try_from_ledger(repr, ctx) {
            let timed_output_ref = ctx.select::<TimedOutputRef>();
            Some(Snapshot(DaoEntity::WPollVoteOrder(order), timed_output_ref))
        } else if let Some(order) = RedeemVotingEscrowOnchainOrder::try_from_ledger(repr, ctx) {
            let timed_output_ref = ctx.select::<TimedOutputRef>();
            Some(Snapshot(
                DaoEntity::RedeemVotingEscrowOrder(order),
                timed_output_ref,
            ))
        } else {
            None
        }
    }
}

#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize, Hash)]
pub struct ProxyOrderMetadata {
    pub signature: Vec<u8>,
    pub witness_script_hash: ScriptHash,
    pub prefix_bytes: Vec<u8>,
    pub postfix_bytes: Vec<u8>,
    pub version: u32,
}

pub fn get_proxy_order_metadata(value: Metadata) -> Option<ProxyOrderMetadata> {
    let extract = |ix| {
        let res = value.get_all(ix)?;
        if res.len() != 1 {
            None
        } else {
            Some((**res.last().unwrap()).clone())
        }
    };
    let message_md = extract(0)?;
    let witness_script_hash_md = extract(1)?;
    let prefix_bytes_md = extract(2)?;
    let postfix_bytes_md = extract(3)?;
    let version_md = extract(4)?;
    if let (
        TransactionMetadatum::Bytes { bytes: message, .. },
        TransactionMetadatum::Bytes {
            bytes: witness_script_hash_bytes,
            ..
        },
        TransactionMetadatum::Bytes {
            bytes: prefix_bytes, ..
        },
        TransactionMetadatum::Bytes {
            bytes: postfix_bytes, ..
        },
        TransactionMetadatum::Int(Int::Uint { value: version, .. }),
    ) = (
        message_md,
        witness_script_hash_md,
        prefix_bytes_md,
        postfix_bytes_md,
        version_md,
    ) {
        let witness_script_hash = ScriptHash::from_raw_bytes(&witness_script_hash_bytes).ok()?;

        return Some(ProxyOrderMetadata {
            signature: message,
            witness_script_hash,
            prefix_bytes,
            postfix_bytes,
            version: version as u32,
        });
    }
    None
}

/// Try to create a ProxyOrderMetadata-compatible Metadata instance from a JSON map. Format of this
/// map is obtained from the Maestro API.
pub fn try_make_proxy_order_metadata_from_json(json: serde_json::Value) -> Option<Metadata> {
    let mut metadata = Metadata::new();
    let signature_hex = json.get("0")?.as_str()?;
    let signature = hex::decode(signature_hex).ok()?;
    let witness_script_hash_hex = json.get("1")?.as_str()?;
    let witness_script_hash = ScriptHash::from_hex(witness_script_hash_hex).ok()?;
    let prefix_bytes_hex = json.get("2")?.as_str()?;
    let prefix_bytes = hex::decode(prefix_bytes_hex).ok()?;
    let postfix_bytes_hex = json.get("3")?.as_str()?;
    let postfix_bytes = hex::decode(postfix_bytes_hex).ok()?;
    let version = json.get("4")?.as_u64()? as u32;

    metadata.set(0, TransactionMetadatum::new_bytes(signature).unwrap());
    metadata.set(
        1,
        TransactionMetadatum::new_bytes(witness_script_hash.to_raw_bytes()).unwrap(),
    );
    metadata.set(2, TransactionMetadatum::new_bytes(prefix_bytes).unwrap());
    metadata.set(3, TransactionMetadatum::new_bytes(postfix_bytes).unwrap());
    metadata.set(4, TransactionMetadatum::new_int(Int::from(version as u64)));
    Some(metadata)
}

impl From<ProxyOrderMetadata> for Metadata {
    fn from(value: ProxyOrderMetadata) -> Self {
        let mut metadata = Metadata::new();
        metadata.set(0, TransactionMetadatum::new_bytes(value.signature).unwrap());
        metadata.set(
            1,
            TransactionMetadatum::new_bytes(value.witness_script_hash.to_raw_bytes()).unwrap(),
        );
        metadata.set(2, TransactionMetadatum::new_bytes(value.prefix_bytes).unwrap());
        metadata.set(3, TransactionMetadatum::new_bytes(value.postfix_bytes).unwrap());
        metadata.set(4, TransactionMetadatum::new_int(Int::from(value.version as u64)));
        metadata
    }
}

/// The plutus-data element that is part of the signature for VE TX authorisation.
pub enum ProxyOrderSignatureElement {
    WPollVote,
    RedeemVE,
}

#[derive(Hash, PartialEq, Eq, Serialize, Deserialize, Clone, Debug, derive_more::From)]
pub enum DaoOrder {
    WPollVote(WPollVoteOnchainOrder),
    MakeVE(MakeVotingEscrowOrder),
    ExtendVE(ExtendVotingEscrowOnchainOrder),
    RedeemVE(RedeemVotingEscrowOnchainOrder),
}

impl DaoOrder {
    pub fn get_owner(&self) -> Owner {
        match self {
            DaoOrder::MakeVE(order) => order.ve_datum.owner,
            DaoOrder::ExtendVE(order) => order.datum.ve_state.owner,
            DaoOrder::WPollVote(order) => order.datum.ve_state.owner,
            DaoOrder::RedeemVE(order) => order.datum.ve_state.owner,
        }
    }
}

#[derive(Hash, PartialEq, Eq, Serialize, Deserialize, Clone, Debug)]
pub struct DaoOrderBundle<Bearer> {
    pub order: DaoOrder,
    pub output_ref: TimedOutputRef,
    pub bearer: Bearer,
}

impl<Bearer> DaoOrderBundle<Bearer> {
    pub fn new(order: DaoOrder, output_ref: TimedOutputRef, bearer: Bearer) -> Self {
        Self {
            order,
            output_ref,
            bearer,
        }
    }
}

impl<Bearer> UniqueOrder for DaoOrderBundle<Bearer> {
    type TOrderId = OutputRef;

    fn get_self_ref(&self) -> Self::TOrderId {
        self.output_ref.output_ref
    }
}

impl<Bearer> Weighted for DaoOrderBundle<Bearer> {
    fn weight(&self) -> OrderWeight {
        // Older orders first
        OrderWeight::from(u64::MAX - self.output_ref.slot.0)
    }
}

impl<Bearer> From<WPollVoteOrderBundle<Bearer>> for DaoOrderBundle<Bearer> {
    fn from(value: WPollVoteOrderBundle<Bearer>) -> Self {
        Self {
            order: DaoOrder::WPollVote(value.order),
            output_ref: value.output_ref,
            bearer: value.bearer,
        }
    }
}

impl<Bearer> From<MakeVotingEscrowOrderBundle<Bearer>> for DaoOrderBundle<Bearer> {
    fn from(value: MakeVotingEscrowOrderBundle<Bearer>) -> Self {
        Self {
            order: DaoOrder::MakeVE(value.order),
            output_ref: value.output_ref,
            bearer: value.bearer,
        }
    }
}

impl<Bearer> From<ExtendVotingEscrowOrderBundle<Bearer>> for DaoOrderBundle<Bearer> {
    fn from(value: ExtendVotingEscrowOrderBundle<Bearer>) -> Self {
        Self {
            order: DaoOrder::ExtendVE(value.order),
            output_ref: value.output_ref,
            bearer: value.bearer,
        }
    }
}

impl<Bearer> From<RedeemVotingEscrowOrderBundle<Bearer>> for DaoOrderBundle<Bearer> {
    fn from(value: RedeemVotingEscrowOrderBundle<Bearer>) -> Self {
        Self {
            order: DaoOrder::RedeemVE(value.order),
            output_ref: value.output_ref,
            bearer: value.bearer,
        }
    }
}
