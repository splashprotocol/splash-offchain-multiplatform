use crate::config::HarvestLimits;
use crate::onchain::event::PollFactoryEvents::{FactoryStateUpdate, NewFactory};
use crate::tx_view::TxViewPartiallyResolved;
use cml_chain::address::Address;
use cml_chain::certs::Credential;
use serde::{Deserialize, Serialize};
use spectrum_cardano_lib::transaction::TransactionOutputExtension;
use spectrum_cardano_lib::{AssetClass, OutputRef, Token};
use spectrum_offchain::domain::{Has, Stable};
use spectrum_offchain::ledger::TryFromLedger;
use spectrum_offchain_cardano::data::pool::{AnyPool, PoolValidation};
use spectrum_offchain_cardano::data::PoolId;
use spectrum_offchain_cardano::deployment::ProtocolValidator::{
    BalanceFnPoolV1, BalanceFnPoolV2, ConstFnPoolFeeSwitch, ConstFnPoolFeeSwitchBiDirFee,
    ConstFnPoolFeeSwitchV2, ConstFnPoolV1, ConstFnPoolV2, RoyaltyPoolV1, StableFnPoolT2T,
};
use spectrum_offchain_cardano::deployment::{test_address, DeployedScriptInfo};
use splash_dao_offchain::deployment::ProtocolValidator;
use splash_dao_offchain::entities::onchain::poll_factory::{PollFactory, PollFactorySnapshot};
use splash_dao_offchain::entities::onchain::smart_farm::{FarmId, SmartFarmSnapshot};
use splash_dao_offchain::protocol_config::{FarmAuthPolicy, PermManagerAuthPolicy, WPFactoryAuthPolicy};
use splash_dao_offchain::routines::{ProvideTimedOref, Slot, TimedOutputRef};
use std::collections::HashSet;

/// Events extracted from on-chain transactions.
#[derive(Serialize, Deserialize, PartialEq, Eq, Debug)]
pub enum StatelessOnChainEvent {
    Position(PositionEvent),
    MultipleHarvest(MultipleAccountsHarvest),
    FarmCreated(FarmCreated),
    PollFactory(PollFactoryEvents),
}

/// Events that happened on-chain but derived from a broad on-chain context.
#[derive(Serialize, Deserialize, PartialEq, Eq, Debug)]
pub enum OnChainEvent {
    Account(AccountEvent),
    FarmEvent(FarmEvent),
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
        + Has<DeployedScriptInfo<{ ProtocolValidator::WpFactory as u8 }>>
        + Has<DeployedScriptInfo<{ ProtocolValidator::SmartFarm as u8 }>>
        + Has<DeployedScriptInfo<{ ProtocolValidator::HarvestOrder as u8 }>>
        + Has<PoolValidation>
        + Has<PermManagerAuthPolicy>
        + Has<WPFactoryAuthPolicy>
        + Has<FarmAuthPolicy>
        + Has<HarvestLimits>,
{
    fn try_from_ledger(repr: &TxViewPartiallyResolved, ctx: &Cx) -> Option<Self> {
        PositionEvent::try_from_ledger(repr, ctx)
            .map(StatelessOnChainEvent::Position)
            .or_else(|| FarmCreated::try_from_ledger(repr, ctx).map(StatelessOnChainEvent::FarmCreated))
            .or_else(|| PollFactoryEvents::try_from_ledger(repr, ctx).map(StatelessOnChainEvent::PollFactory))
            .or_else(|| {
                MultipleAccountsHarvest::try_from_ledger(repr, ctx)
                    .map(StatelessOnChainEvent::MultipleHarvest)
            })
    }
}

impl OnChainEvent {
    pub fn pool_id(&self) -> PoolId {
        match self {
            OnChainEvent::Account(dr) => dr.pool_id(),
            OnChainEvent::FarmEvent(fe) => fe.pool_id(),
        }
    }
}

#[derive(Serialize, Deserialize, PartialEq, Eq, Debug)]
pub enum AccountEvent {
    Position(PositionEvent),
    Harvest(Harvest),
}

impl AccountEvent {
    pub fn pool_id(&self) -> PoolId {
        match self {
            AccountEvent::Position(d) => d.pool_id(),
            AccountEvent::Harvest(h) => h.pool_id,
        }
    }
    pub fn account(&self) -> Credential {
        match self {
            AccountEvent::Position(d) => d.account(),
            AccountEvent::Harvest(h) => h.account.clone(),
        }
    }
}

#[derive(Serialize, Deserialize, PartialEq, Eq, Debug)]
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
        if let Some(pool) = PoolDiff::try_from_ledger(repr, ctx) {
            let (plus_sign, diff) = pool.lp_diff;
            if diff != 0 {
                if let Some(account) = find_lp_recv(pool.lp_asset.into_token().unwrap(), pool.pool_id, repr) {
                    let account = account.payment_cred().unwrap().clone();
                    return Some(if plus_sign {
                        PositionEvent::Deposit(Deposit {
                            pool_id: pool.pool_id,
                            account,
                            lp_mint: diff,
                            lp_supply: pool.lp_supply,
                        })
                    } else {
                        PositionEvent::Redeem(Redeem {
                            pool_id: pool.pool_id,
                            account,
                            lp_burned: diff,
                            lp_supply: pool.lp_supply,
                        })
                    });
                }
            }
        }
        None
    }
}

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
            maybe_utxo.as_ref().and_then(|u| AnyPool::try_from_ledger(u, ctx))
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

fn find_lp_recv(Token(pol, tn): Token, PoolId(Token(pool_nft_pol, pool_nft_tn)): PoolId, tx: &TxViewPartiallyResolved) -> Option<Address> {
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

#[derive(Serialize, Deserialize, PartialEq, Eq, Debug)]
pub struct Harvest {
    pub pool_id: PoolId,
    pub account: Credential,
    pub harvested_till: cml_chain::Slot,
}

#[derive(Serialize, Deserialize, PartialEq, Eq, Debug)]
pub struct MultipleAccountsHarvest {
    pub accounts: Vec<Credential>,
    pub harvested_till: Slot,
}

impl<Cx> TryFromLedger<TxViewPartiallyResolved, Cx> for MultipleAccountsHarvest
where
    Cx: Has<DeployedScriptInfo<{ ProtocolValidator::HarvestOrder as u8 }>> + Has<HarvestLimits>,
{
    fn try_from_ledger(repr: &TxViewPartiallyResolved, ctx: &Cx) -> Option<Self> {
        repr.outputs.iter().find_map(|output| {
            let correct_lovelace_value = output.value().coin as u64
                >= (repr.signers.len() as u64
                    * ctx.select::<HarvestLimits>().minimal_lovelace_per_single_harvest);
            if test_address(output.address(), ctx) && correct_lovelace_value {
                let accounts = repr
                    .signers
                    .clone()
                    .into_iter()
                    .map(Credential::new_pub_key)
                    .collect();
                Some(MultipleAccountsHarvest {
                    accounts,
                    harvested_till: Slot(repr.slot),
                })
            } else {
                None
            }
        })
    }
}

#[derive(Serialize, Deserialize, PartialEq, Eq, Debug)]
pub struct FarmCreated {
    pub farm_id: FarmId,
    pub pool_id: PoolId,
}

impl<Cx> TryFromLedger<TxViewPartiallyResolved, Cx> for FarmCreated
where
    Cx: Has<PermManagerAuthPolicy>
        + Has<FarmAuthPolicy>
        + Has<DeployedScriptInfo<{ ProtocolValidator::SmartFarm as u8 }>>,
{
    fn try_from_ledger(repr: &TxViewPartiallyResolved, ctx: &Cx) -> Option<Self> {
        let farms_in_inputs: HashSet<_> =
            HashSet::from_iter(repr.inputs.iter().filter_map(|(i, maybe_utxo)| {
                maybe_utxo
                    .as_ref()
                    .and_then(|u| {
                        let oref = TimedOutputRef::new(OutputRef::from(i.clone()), Slot(0));
                        SmartFarmSnapshot::try_from_ledger(u, &ProvideTimedOref(ctx, oref))
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
        new_farms.next().map(|sm| FarmCreated {
            farm_id: sm.farm_id,
            pool_id: sm.pool_id,
        })
    }
}

#[derive(Serialize, Deserialize, PartialEq, Eq, Debug)]
pub enum PollFactoryEvents {
    NewFactory(PollFactory),
    FactoryStateUpdate(PollFactoryUpdated),
}

impl<Cx> TryFromLedger<TxViewPartiallyResolved, Cx> for PollFactoryEvents
where
    Cx: Has<DeployedScriptInfo<{ ProtocolValidator::WpFactory as u8 }>> + Has<WPFactoryAuthPolicy>,
{
    fn try_from_ledger(repr: &TxViewPartiallyResolved, ctx: &Cx) -> Option<Self> {
        let factory_in_inputs: HashSet<_> =
            HashSet::from_iter(repr.inputs.iter().filter_map(|(i, maybe_utxo)| {
                maybe_utxo
                    .as_ref()
                    .and_then(|u| {
                        let oref = TimedOutputRef::new(OutputRef::from(i.clone()), Slot(0));
                        PollFactorySnapshot::try_from_ledger(u, &ProvideTimedOref(ctx, oref))
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

#[derive(Serialize, Deserialize, PartialEq, Eq, Debug)]
pub struct PollFactoryUpdated {
    pub new_state: PollFactory,
}

#[derive(Serialize, Deserialize, PartialEq, Eq, Debug)]
pub enum FarmEvent {
    FarmActivated(FarmActivated),
    FarmDeactivated(FarmDeactivated),
}

impl FarmEvent {
    pub fn pool_id(&self) -> PoolId {
        match self {
            FarmEvent::FarmActivated(a) => a.pool_id,
            FarmEvent::FarmDeactivated(d) => d.pool_id,
        }
    }
}

#[derive(Serialize, Deserialize, PartialEq, Eq, Debug)]
pub struct FarmActivated {
    pub pool_id: PoolId,
}

#[derive(Serialize, Deserialize, PartialEq, Eq, Debug)]
pub struct FarmDeactivated {
    pub pool_id: PoolId,
}
