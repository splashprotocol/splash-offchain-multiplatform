use crate::{settings::MinLovelacePerHarvest, Epoch};
use cml_chain::{
    certs::{Credential, StakeCredential},
    plutus::{ConstrPlutusData, PlutusData},
    transaction::TransactionOutput,
};
use cml_crypto::{Ed25519KeyHash, RawBytesEncoding};
use serde::{Deserialize, Serialize};
use spectrum_cardano_lib::{
    address::PlutusAddress,
    output::FinalizedTxOut,
    plutus_data::{ConstrPlutusDataExtension, DatumExtension, IntoPlutusData, PlutusDataExtension},
    transaction::TransactionOutputExtension,
    tx_view::{TimedOutput, TxViewPartiallyResolved},
    types::TryFromPData,
    NetworkId, OutputRef,
};
use spectrum_offchain::domain::Has;
use spectrum_offchain_cardano::deployment::{test_address, DeployedScriptInfo};
use splash_dao_offchain::{
    deployment::ProtocolValidator as DaoProtocolValidator,
    routines::{slot_to_epoch, Slot},
    GenesisEpochStartTime,
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HarvestOrder<OrderId> {
    pub id: OrderId,
    /// Key that signs harvest
    pub account_key: Ed25519KeyHash,
    pub issued_at: (Slot, Epoch),
    /// Where the reward should be sent
    pub reward_receiver: PlutusAddress,
}

#[derive(Debug, Clone, PartialEq)]
pub struct HarvestOrderCredential(Credential);

#[derive(Debug, Clone, PartialEq)]
pub struct HarvestOrderDatum {
    /// Key that signs harvest
    account_key: Ed25519KeyHash,
    /// Where the reward should be sent
    reward_receiver: PlutusAddress,
    distribution_agent_key: Ed25519KeyHash,
}

impl TryFromPData for HarvestOrderDatum {
    fn try_from_pd(data: PlutusData) -> Option<Self> {
        let mut cpd = data.into_constr_pd()?;
        let account_key = cpd
            .take_field(0)?
            .into_bytes()
            .map(|bytes| Ed25519KeyHash::from_raw_bytes(&bytes).ok())??;

        let reward_receiver = PlutusAddress::try_from_pd(cpd.take_field(1)?)?;
        let distribution_agent_key = cpd
            .take_field(2)?
            .into_bytes()
            .map(|bytes| Ed25519KeyHash::from_raw_bytes(&bytes).ok())??;
        Some(Self {
            account_key,
            reward_receiver,
            distribution_agent_key,
        })
    }
}

pub enum HarvestOrderAction {
    Refund,
    Harvest,
}

impl IntoPlutusData for HarvestOrderAction {
    fn into_pd(self) -> PlutusData {
        let alternative = match self {
            HarvestOrderAction::Refund => 0,
            HarvestOrderAction::Harvest => 1,
        };
        PlutusData::ConstrPlutusData(ConstrPlutusData::new(alternative, vec![]))
    }
}

/// Try to extract a newly-created harvest order. If multiple orders exist in this TX we take the
/// first one and ignore subsequent orders.
pub(crate) fn try_new_harvest_request<C>(
    repr: &TxViewPartiallyResolved,
    ctx: &C,
) -> Option<(HarvestOrder<OutputRef>, FinalizedTxOut)>
where
    C: Has<MinLovelacePerHarvest>
        + Has<GenesisEpochStartTime>
        + Has<NetworkId>
        + Has<DeployedScriptInfo<{ DaoProtocolValidator::HarvestOrder as u8 }>>,
{
    repr.outputs.iter().enumerate().find_map(|(ix, output)| {
        let output_ref = OutputRef::new(repr.hash, ix as u64);
        let genesis_epoch_start_time = ctx.select::<GenesisEpochStartTime>();
        let network_id = ctx.select::<NetworkId>();
        let epoch = slot_to_epoch(repr.slot, genesis_epoch_start_time, network_id).0 as u64;
        try_extract_harvest_order(output, output_ref, (Slot(repr.slot), Epoch::from(epoch)), ctx)
            .map(|order| (order, FinalizedTxOut(output.clone(), output_ref)))
    })
}

/// Returns the OutputRefs of all known harvest orders that have been consumed.
pub(crate) fn get_consumed_harvest_orders<C>(
    repr: &TxViewPartiallyResolved,
    ctx: &C,
) -> Vec<HarvestOrder<OutputRef>>
where
    C: Has<MinLovelacePerHarvest>
        + Has<GenesisEpochStartTime>
        + Has<NetworkId>
        + Has<DeployedScriptInfo<{ DaoProtocolValidator::HarvestOrder as u8 }>>,
{
    repr.inputs
        .iter()
        .filter_map(|(tx_input, output)| {
            if let Some(TimedOutput { output, slot }) = output {
                let output_ref = OutputRef::from(tx_input.clone());
                let genesis_epoch_start_time = ctx.select::<GenesisEpochStartTime>();
                let network_id = ctx.select::<NetworkId>();
                let epoch = slot_to_epoch(*slot, genesis_epoch_start_time, network_id).0 as u64;
                return try_extract_harvest_order(output, output_ref, (Slot(*slot), Epoch::from(epoch)), ctx);
            }
            None
        })
        .collect()
}

pub fn try_extract_harvest_order<C>(
    output: &TransactionOutput,
    output_ref: OutputRef,
    issued_at: (Slot, Epoch),
    ctx: &C,
) -> Option<HarvestOrder<OutputRef>>
where
    C: Has<MinLovelacePerHarvest> + Has<DeployedScriptInfo<{ DaoProtocolValidator::HarvestOrder as u8 }>>,
{
    let harvest_limit = ctx.select::<MinLovelacePerHarvest>();
    let lovelace_amount = output.value().coin;
    if test_address(output.address(), ctx) && lovelace_amount >= harvest_limit.0 {
        let datum = output.datum()?;
        let HarvestOrderDatum {
            account_key,
            reward_receiver,
            ..
        } = datum.into_pd().map(HarvestOrderDatum::try_from_pd)??;
        let harvest_order = HarvestOrder {
            id: output_ref,
            account_key,
            issued_at,
            reward_receiver,
        };
        return Some(harvest_order);
    }
    None
}
