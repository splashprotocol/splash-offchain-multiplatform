use spectrum_offchain::display::display_tuple;
use spectrum_offchain::display::display_vec;
use crate::onchain::event::PollFactoryEvents::{FactoryStateUpdate, NewFactory};
use cml_chain::address::Address;
use cml_chain::certs::Credential;
use derive_more::Display;
use serde::{Deserialize, Serialize};
use spectrum_cardano_lib::transaction::TransactionOutputExtension;
use spectrum_cardano_lib::tx_view::{TimedOutput, TxViewPartiallyResolved};
use spectrum_cardano_lib::{AssetClass, NetworkId, OutputRef, Token};
use spectrum_offchain::domain::{Has, Stable};
use spectrum_offchain::ledger::TryFromLedger;
use spectrum_offchain_cardano::data::pool::{AnyPool, PoolValidation};
use spectrum_offchain_cardano::data::PoolId;
use spectrum_offchain_cardano::deployment::DeployedScriptInfo;
use spectrum_offchain_cardano::deployment::ProtocolValidator::{
    BalanceFnPoolV1, BalanceFnPoolV2, ConstFnPoolFeeSwitch, ConstFnPoolFeeSwitchBiDirFee,
    ConstFnPoolFeeSwitchV2, ConstFnPoolV1, ConstFnPoolV2, RoyaltyPoolV1, StableFnPoolT2T,
};
use splash_dao_offchain::deployment::ProtocolValidator as DaoProtocolValidator;
use splash_dao_offchain::entities::onchain::poll_factory::{PollFactory, PollFactorySnapshot};
use splash_dao_offchain::entities::onchain::smart_farm::{FarmId, SmartFarmSnapshot};
use splash_dao_offchain::protocol_config::{
    BufferWalletScript, OperatorCreds, PermManagerAuthPolicy, SplashPolicy, WPFactoryAuthPolicy,
};
use splash_dao_offchain::routines::{ProvideTimedOref, Slot, TimedOutputRef};
use splash_yf_offchain::events::OnChainEvent as RewardOnChainEvent;
use splash_yf_offchain::settings::MinLovelacePerHarvest;
use splash_yf_offchain::Epoch;
use std::collections::HashSet;
use std::fmt::{Display, Formatter};

/// Events extracted from on-chain transactions.
#[derive(Serialize, Deserialize, PartialEq, Eq, Debug)]
pub enum StatelessOnChainEvent {
    Position(PositionEvent),
    Gauge(GaugeCreated),
    Pool(PoolCreated),
    WeightingPoll(WeightingPollCompleted) // todo: add parser
}

/// Events that happened on-chain but derived from a broad on-chain context.
#[derive(Serialize, Deserialize, PartialEq, Eq, Debug, Display)]
pub enum OnChainEvent {
    Account(PositionEvent),
    Gauge(GaugeEvent),
    Pool(PoolCreated),
}

impl<Cx> TryFromLedger<TxViewPartiallyResolved, Cx> for StatelessOnChainEvent
where
    Cx: Has<DeployedScriptInfo<{ ConstFnPoolV1 as u8 }>>
        + Has<DeployedScriptInfo<{ ConstFnPoolV2 as u8 }>>
        + Has<DeployedScriptInfo<{ ConstFnPoolFeeSwitch as u8 }>>
        + Has<DeployedScriptInfo<{ ConstFnPoolFeeSwitchV2 as u8 }>>
        + Has<DeployedScriptInfo<{ ConstFnPoolFeeSwitchBiDirFee as u8 }>>
        + Has<DeployedScriptInfo<{ BalanceFnPoolV1 as u8 }>>
        + Has<DeployedScriptInfo<{ BalanceFnPoolV2 as u8 }>>
        + Has<DeployedScriptInfo<{ StableFnPoolT2T as u8 }>>
        + Has<DeployedScriptInfo<{ RoyaltyPoolV1 as u8 }>>
        + Has<DeployedScriptInfo<{ DaoProtocolValidator::WpFactory as u8 }>>
        + Has<DeployedScriptInfo<{ DaoProtocolValidator::SmartFarm as u8 }>>
        + Has<DeployedScriptInfo<{ DaoProtocolValidator::HarvestOrder as u8 }>>
        + Has<DeployedScriptInfo<{ DaoProtocolValidator::PermManager as u8 }>>
        + Has<BufferWalletScript>
        + Has<PoolValidation>
        + Has<PermManagerAuthPolicy>
        + Has<WPFactoryAuthPolicy>
        + Has<SplashPolicy>
        + Has<OperatorCreds>
        + Has<NetworkId>
        + Has<MinLovelacePerHarvest>,
{
    fn try_from_ledger(repr: &TxViewPartiallyResolved, ctx: &Cx) -> Option<Self> {
        PositionEvent::try_from_ledger(repr, ctx)
            .map(StatelessOnChainEvent::Position)
            .or_else(|| GaugeCreated::try_from_ledger(repr, ctx).map(StatelessOnChainEvent::Gauge))
            .or_else(|| PoolCreated::try_from_ledger(repr, ctx).map(StatelessOnChainEvent::Pool))
    }
}

impl OnChainEvent {
    pub fn pool_id(&self) -> PoolId {
        match self {
            OnChainEvent::Account(dr) => dr.pool_id(),
            OnChainEvent::Gauge(fe) => fe.pool_id(),
            OnChainEvent::Pool(fe) => fe.pool_id,
        }
    }
}

#[derive(Serialize, Deserialize, PartialEq, Eq, Debug, Display)]
pub enum PositionEvent {
    Deposit(Deposit),
    Redeem(Redeem),
}

impl PositionEvent {
    pub fn pool_id(&self) -> PoolId {
        match self {
            PositionEvent::Deposit(d) => d.pool_id,
            PositionEvent::Redeem(r) => r.pool_id,
        }
    }
    pub fn account(&self) -> Credential {
        match self {
            PositionEvent::Deposit(d) => d.account.clone(),
            PositionEvent::Redeem(r) => r.account.clone(),
        }
    }
    pub fn lp_supply(&self) -> u64 {
        match self {
            PositionEvent::Deposit(d) => d.lp_supply,
            PositionEvent::Redeem(r) => r.lp_supply,
        }
    }
}

impl<Cx> TryFromLedger<TxViewPartiallyResolved, Cx> for PositionEvent
where
    Cx: Has<DeployedScriptInfo<{ ConstFnPoolV1 as u8 }>>
        + Has<DeployedScriptInfo<{ ConstFnPoolV2 as u8 }>>
        + Has<DeployedScriptInfo<{ ConstFnPoolFeeSwitch as u8 }>>
        + Has<DeployedScriptInfo<{ ConstFnPoolFeeSwitchV2 as u8 }>>
        + Has<DeployedScriptInfo<{ ConstFnPoolFeeSwitchBiDirFee as u8 }>>
        + Has<DeployedScriptInfo<{ BalanceFnPoolV1 as u8 }>>
        + Has<DeployedScriptInfo<{ BalanceFnPoolV2 as u8 }>>
        + Has<DeployedScriptInfo<{ StableFnPoolT2T as u8 }>>
        + Has<DeployedScriptInfo<{ RoyaltyPoolV1 as u8 }>>
        + Has<PoolValidation>,
{
    fn try_from_ledger(repr: &TxViewPartiallyResolved, ctx: &Cx) -> Option<Self> {
        if let Some(pool_diff) = PoolDiff::try_from_ledger(repr, ctx) {
            let (plus_sign, diff) = pool_diff.lp_diff;
            if diff != 0 {
                if let Some(account) =
                    find_lp_recv(pool_diff.lp_asset.into_token().unwrap(), pool_diff.pool_id, repr)
                {
                    let account = account.payment_cred().unwrap().clone();
                    return Some(if plus_sign {
                        PositionEvent::Deposit(Deposit {
                            pool_id: pool_diff.pool_id,
                            account,
                            lp_mint: diff,
                            lp_supply: pool_diff.lp_supply,
                        })
                    } else {
                        PositionEvent::Redeem(Redeem {
                            pool_id: pool_diff.pool_id,
                            account,
                            lp_burned: diff,
                            lp_supply: pool_diff.lp_supply,
                        })
                    });
                }
            }
        }
        None
    }
}

#[derive(Debug)]
struct PoolDiff {
    pool_id: PoolId,
    lp_asset: AssetClass,
    lp_diff: (bool, u64),
    lp_supply: u64,
}

impl<Cx> TryFromLedger<TxViewPartiallyResolved, Cx> for PoolDiff
where
    Cx: Has<DeployedScriptInfo<{ ConstFnPoolV1 as u8 }>>
        + Has<DeployedScriptInfo<{ ConstFnPoolV2 as u8 }>>
        + Has<DeployedScriptInfo<{ ConstFnPoolFeeSwitch as u8 }>>
        + Has<DeployedScriptInfo<{ ConstFnPoolFeeSwitchV2 as u8 }>>
        + Has<DeployedScriptInfo<{ ConstFnPoolFeeSwitchBiDirFee as u8 }>>
        + Has<DeployedScriptInfo<{ BalanceFnPoolV1 as u8 }>>
        + Has<DeployedScriptInfo<{ BalanceFnPoolV2 as u8 }>>
        + Has<DeployedScriptInfo<{ StableFnPoolT2T as u8 }>>
        + Has<DeployedScriptInfo<{ RoyaltyPoolV1 as u8 }>>
        + Has<PoolValidation>,
{
    fn try_from_ledger(repr: &TxViewPartiallyResolved, ctx: &Cx) -> Option<Self> {
        let pool_in = repr.inputs.iter().find_map(|(input, maybe_utxo)| {
            maybe_utxo
                .as_ref()
                .and_then(|TimedOutput { output, .. }| AnyPool::try_from_ledger(output, ctx))
        });
        let pool_out = repr.outputs.iter().find_map(|u| AnyPool::try_from_ledger(u, ctx));
        if let (Some(pin), Some(pout)) = (pool_in, pool_out) {
            let (lp_in, lp_asset) = match pin {
                AnyPool::PureCFMM(p) => (p.liquidity.untag(), p.asset_lq.untag()),
                AnyPool::BalancedCFMM(p) => (p.liquidity.untag(), p.asset_lq.untag()),
                AnyPool::StableCFMM(p) => (p.liquidity.untag(), p.asset_lq.untag()),
            };
            let lp_out = match pout {
                AnyPool::PureCFMM(p) => p.liquidity,
                AnyPool::BalancedCFMM(p) => p.liquidity,
                AnyPool::StableCFMM(p) => p.liquidity,
            }
            .untag();
            let lp_diff = lp_out
                .checked_sub(lp_in)
                .map(|r| (true, r))
                .unwrap_or_else(|| (false, lp_in - lp_out));
            return Some(PoolDiff {
                pool_id: pin.stable_id().into(),
                lp_asset,
                lp_diff,
                lp_supply: lp_out,
            });
        }
        None
    }
}

#[derive(Serialize, Deserialize, PartialEq, Eq, Debug)]
pub struct Deposit {
    pub pool_id: PoolId,
    pub account: Credential,
    pub lp_mint: u64,
    pub lp_supply: u64,
}

impl Display for Deposit {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        let account = hex::encode(self.account.to_raw_bytes());
        write!(
            f,
            "Deposit (pool_id: {}, account: {:?}, lp_mint: {}, lp_supply: {})",
            self.pool_id, account, self.lp_mint, self.lp_supply
        )
    }
}

fn find_lp_recv(
    Token(pol, tn): Token,
    PoolId(Token(pool_nft_pol, pool_nft_tn)): PoolId,
    tx: &TxViewPartiallyResolved,
) -> Option<Address> {
    tx.outputs.iter().find_map(|output| {
        if output.value().multiasset.get(&pol, &tn.into()).is_some()
            && output
                .value()
                .multiasset
                .get(&pool_nft_pol, &pool_nft_tn.into())
                .is_none()
        {
            Some(output.address().clone())
        } else {
            None
        }
    })
}

#[derive(Serialize, Deserialize, PartialEq, Eq, Debug)]
pub struct Redeem {
    pub pool_id: PoolId,
    pub account: Credential,
    pub lp_burned: u64,
    pub lp_supply: u64,
}

impl Display for Redeem {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        let account = hex::encode(self.account.to_raw_bytes());
        write!(
            f,
            "Redeem (pool_id: {}, account: {:?}, lp_burned: {}, lp_supply: {})",
            self.pool_id, account, self.lp_burned, self.lp_supply
        )
    }
}

#[derive(Serialize, Deserialize, PartialEq, Eq, Debug)]
pub struct AccountPoolHarvested {
    pub pool_id: PoolId,
    pub account: Credential,
    pub harvested_till: cml_chain::Slot,
}

impl Display for AccountPoolHarvested {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        let account = hex::encode(self.account.to_raw_bytes());
        write!(
            f,
            "Harvest(pool_id = {}, account = {}, harvested_till = {})",
            self.pool_id, account, self.harvested_till
        )
    }
}

/// Harvest has been executed on-chain.
#[derive(Serialize, Deserialize, PartialEq, Eq, Debug)]
pub struct MultiAccountHarvested {
    pub accounts: Vec<Credential>,
    pub harvested_till: Slot,
}

impl<Cx> TryFromLedger<TxViewPartiallyResolved, Cx> for MultiAccountHarvested
where
    Cx: Has<PermManagerAuthPolicy>
        + Has<SplashPolicy>
        + Has<PermManagerAuthPolicy>
        + Has<MinLovelacePerHarvest>
        + Has<OperatorCreds>
        + Has<NetworkId>
        + Has<BufferWalletScript>
        + Has<DeployedScriptInfo<{ DaoProtocolValidator::SmartFarm as u8 }>>
        + Has<DeployedScriptInfo<{ DaoProtocolValidator::PermManager as u8 }>>
        + Has<DeployedScriptInfo<{ DaoProtocolValidator::HarvestOrder as u8 }>>,
{
    fn try_from_ledger(repr: &TxViewPartiallyResolved, ctx: &Cx) -> Option<Self> {
        let reward_event = RewardOnChainEvent::try_from_ledger(repr, ctx)?;

        if let RewardOnChainEvent::BotHarvestingAction { payouts, .. } = reward_event {
            let mut most_recent_slot = 0;
            let accounts: Vec<_> = payouts
                .iter()
                .map(|(harvest_order, _)| {
                    let issued_at = harvest_order.issued_at.0;
                    if issued_at > most_recent_slot {
                        most_recent_slot = issued_at;
                    }
                    Credential::new_pub_key(harvest_order.account_key)
                })
                .collect();

            if !accounts.is_empty() {
                return Some(Self {
                    accounts,
                    harvested_till: Slot(most_recent_slot),
                });
            }
        }
        None
    }
}

#[derive(Serialize, Deserialize, PartialEq, Eq, Debug, Display)]
#[display(
    "FarmCreated ( farm_id = {}, pool_id = {})",
    farm_id,
    pool_id
)]
pub struct GaugeCreated {
    pub farm_id: FarmId,
    pub pool_id: PoolId,
}

impl<Cx> TryFromLedger<TxViewPartiallyResolved, Cx> for GaugeCreated
where
    Cx: Has<PermManagerAuthPolicy> + Has<DeployedScriptInfo<{ DaoProtocolValidator::SmartFarm as u8 }>>,
{
    fn try_from_ledger(repr: &TxViewPartiallyResolved, ctx: &Cx) -> Option<Self> {
        let farms_in_inputs: HashSet<_> =
            HashSet::from_iter(repr.inputs.iter().filter_map(|(i, maybe_utxo)| {
                maybe_utxo
                    .as_ref()
                    .and_then(|TimedOutput { output, .. }| {
                        let oref = TimedOutputRef::new(OutputRef::from(i.clone()), Slot(0));
                        SmartFarmSnapshot::try_from_ledger(output, &ProvideTimedOref(ctx, oref))
                    })
                    .map(|farm| farm.get().farm_id)
            }));
        let farms_in_outputs = repr.outputs.iter().enumerate().filter_map(|(ix, utxo)| {
            let oref = TimedOutputRef::new(OutputRef::new(repr.hash, ix as u64), Slot(0));
            SmartFarmSnapshot::try_from_ledger(utxo, &ProvideTimedOref(ctx, oref))
        });
        let mut new_farms = farms_in_outputs.filter_map(|farm| {
            if !farms_in_inputs.contains(&farm.get().farm_id) {
                Some(farm.unwrap())
            } else {
                None
            }
        });
        new_farms.next().map(|sm| GaugeCreated {
            farm_id: sm.farm_id,
            pool_id: sm.pool_id,
        })
    }
}

#[derive(Serialize, Deserialize, PartialEq, Eq, Debug, Display)]
pub enum PollFactoryEvents {
    NewFactory(PollFactory),
    FactoryStateUpdate(PollFactoryUpdated),
}

impl<Cx> TryFromLedger<TxViewPartiallyResolved, Cx> for PollFactoryEvents
where
    Cx: Has<DeployedScriptInfo<{ DaoProtocolValidator::WpFactory as u8 }>> + Has<WPFactoryAuthPolicy>,
{
    fn try_from_ledger(repr: &TxViewPartiallyResolved, ctx: &Cx) -> Option<Self> {
        let factory_in_inputs: HashSet<_> =
            HashSet::from_iter(repr.inputs.iter().filter_map(|(i, maybe_utxo)| {
                maybe_utxo
                    .as_ref()
                    .and_then(|TimedOutput { output, .. }| {
                        let oref = TimedOutputRef::new(OutputRef::from(i.clone()), Slot(0));
                        PollFactorySnapshot::try_from_ledger(output, &ProvideTimedOref(ctx, oref))
                    })
                    .map(|farm| farm.get().stable_id)
            }));
        repr.outputs
            .iter()
            .enumerate()
            .filter_map(|(ix, utxo)| {
                let oref = TimedOutputRef::new(OutputRef::new(repr.hash, ix as u64), Slot(0));
                PollFactorySnapshot::try_from_ledger(utxo, &ProvideTimedOref(ctx, oref)).map(|snapshot| {
                    if factory_in_inputs.contains(&snapshot.get().stable_id) {
                        FactoryStateUpdate(PollFactoryUpdated {
                            new_state: snapshot.get().clone(),
                        })
                    } else {
                        NewFactory(snapshot.get().clone())
                    }
                })
            })
            .next()
    }
}

#[derive(Serialize, Deserialize, PartialEq, Eq, Debug, Display)]
pub struct PollFactoryUpdated {
    pub new_state: PollFactory,
}

#[derive(Serialize, Deserialize, PartialEq, Eq, Debug, Display)]
pub enum GaugeEvent {
    GaugeCreated(GaugeCreated),
    GaugeWeighted(GaugeWeighted),
}

impl GaugeEvent {
    pub fn pool_id(&self) -> PoolId {
        match self {
            GaugeEvent::GaugeCreated(a) => a.pool_id,
            GaugeEvent::GaugeWeighted(d) => d.pool_id,
        }
    }
}

#[derive(Serialize, Deserialize, PartialEq, Eq, Debug, Display)]
#[display(
    "GaugeWeighted ( pool_id = {}, weight = {}, epoch = {})",
    pool_id,
    weight,
    epoch
)]
pub struct GaugeWeighted {
    pub pool_id: PoolId,
    pub weight: u64,
    pub epoch: Epoch,
}

#[derive(Serialize, Deserialize, PartialEq, Eq, Debug, Display)]
#[display("WeightingPollCompleted (distribution = {}, epoch = {})", display_vec(&distribution.iter().map(|x| display_tuple(*x)).collect::<Vec<_>>()), epoch)]
pub struct WeightingPollCompleted {
    pub distribution: Vec<(FarmId, u64)>,
    pub epoch: Epoch,
}

#[derive(Serialize, Deserialize, PartialEq, Eq, Debug, Display)]
#[display("PoolCreated (pool_id = {}, supply_lq = {})", pool_id, supply_lq)]
pub struct PoolCreated {
    pub pool_id: PoolId,
    pub supply_lq: u64,
}

impl<Cx> TryFromLedger<TxViewPartiallyResolved, Cx> for PoolCreated
where
    Cx: Has<DeployedScriptInfo<{ ConstFnPoolV1 as u8 }>>
        + Has<DeployedScriptInfo<{ ConstFnPoolV2 as u8 }>>
        + Has<DeployedScriptInfo<{ ConstFnPoolFeeSwitch as u8 }>>
        + Has<DeployedScriptInfo<{ ConstFnPoolFeeSwitchV2 as u8 }>>
        + Has<DeployedScriptInfo<{ ConstFnPoolFeeSwitchBiDirFee as u8 }>>
        + Has<DeployedScriptInfo<{ BalanceFnPoolV1 as u8 }>>
        + Has<DeployedScriptInfo<{ BalanceFnPoolV2 as u8 }>>
        + Has<DeployedScriptInfo<{ StableFnPoolT2T as u8 }>>
        + Has<DeployedScriptInfo<{ RoyaltyPoolV1 as u8 }>>
        + Has<PoolValidation>,
{
    fn try_from_ledger(repr: &TxViewPartiallyResolved, ctx: &Cx) -> Option<Self> {
        let pool_in = repr.inputs.iter().find_map(|(input, maybe_utxo)| {
            maybe_utxo
                .as_ref()
                .and_then(|TimedOutput { output, .. }| AnyPool::try_from_ledger(output, ctx))
        });
        let pool_out = repr.outputs.iter().find_map(|u| AnyPool::try_from_ledger(u, ctx));
        if let (None, Some(pout)) = (pool_in, pool_out) {
            let lp_out = match pout {
                AnyPool::PureCFMM(p) => p.liquidity,
                AnyPool::BalancedCFMM(p) => p.liquidity,
                AnyPool::StableCFMM(p) => p.liquidity,
            };
            return Some(PoolCreated {
                pool_id: pout.stable_id().into(),
                supply_lq: lp_out.untag(),
            });
        }
        None
    }
}
