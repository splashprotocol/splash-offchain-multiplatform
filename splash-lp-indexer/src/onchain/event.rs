use crate::config::HarvestLimits;
use crate::onchain::event::PollFactoryEvents::{FactoryStateUpdate, NewFactory};
use crate::tx_view::TxViewPartiallyResolved;
use cml_chain::address::Address;
use cml_chain::certs::Credential;
use cml_chain::transaction::TransactionOutput;
use cml_crypto::Ed25519KeyHash;
use derive_more::Display;
use log::info;
use log::kv::Source;
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
use splash_dao_offchain::entities::Snapshot;
use splash_dao_offchain::protocol_config::{FarmAuthPolicy, PermManagerAuthPolicy, WPFactoryAuthPolicy};
use splash_dao_offchain::routines::{ProvideTimedOref, Slot, TimedOutputRef};
use std::collections::HashSet;
use std::fmt::Formatter;

/// Events extracted from on-chain transactions.
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq, Debug)]
pub enum StatelessOnChainEvent {
    Position(PositionEvent),
    MultipleHarvest(MultipleAccountsHarvest),
    FarmCreated(FarmCreated),
    PollFactory(PollFactoryEvents),
    PoolCreated(PoolCreated),
}

pub trait WithOptionalSlot {
    fn slot(&self) -> Option<Slot>;
}

/// Events that happened on-chain but derived from a broad on-chain context.
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq, Debug, Display)]
pub enum OnChainEvent {
    Account(AccountEvent),
    FarmEvent(FarmEvent),
    PoolEvent(PoolEvent),
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
        info!(
            "[On-chain event] Testing tx: {}. Inputs: {}",
            repr.hash,
            repr.inputs
                .clone()
                .iter()
                .map(|(tx_input, output)| { format!("{:?} -> {:?},", tx_input, output.is_some()) })
                .fold(String::new(), |acc, x| acc + &x)
        );
        let event = PositionEvent::try_from_ledger(repr, ctx)
            .map(StatelessOnChainEvent::Position)
            .or_else(|| FarmCreated::try_from_ledger(repr, ctx).map(StatelessOnChainEvent::FarmCreated))
            .or_else(|| PollFactoryEvents::try_from_ledger(repr, ctx).map(StatelessOnChainEvent::PollFactory))
            .or_else(|| {
                MultipleAccountsHarvest::try_from_ledger(repr, ctx)
                    .map(StatelessOnChainEvent::MultipleHarvest)
            })
            .or_else(|| PoolCreated::try_from_ledger(repr, ctx).map(StatelessOnChainEvent::PoolCreated));
        info!("[On-chain event] event: {:?}", event);
        event
    }
}

impl WithOptionalSlot for OnChainEvent {
    fn slot(&self) -> Option<Slot> {
        match self {
            OnChainEvent::FarmEvent(FarmEvent::FarmActivated(event)) => Some(event.slot),
            _ => None,
        }
    }
}

impl OnChainEvent {
    pub fn slot(&self) -> Option<Slot> {
        match self {
            OnChainEvent::FarmEvent(FarmEvent::FarmActivated(event)) => Some(event.slot),
            _ => None,
        }
    }

    pub fn pool_id(&self) -> PoolId {
        match self {
            OnChainEvent::Account(dr) => dr.pool_id(),
            OnChainEvent::FarmEvent(fe) => fe.pool_id(),
            OnChainEvent::PoolEvent(fe) => fe.pool_id(),
        }
    }
}

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq, Debug, Display)]
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

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq, Debug, Display)]
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
        info!("[On-chain event] Testing PositionEvent");
        if let Some(pool_diff) = PoolDiff::try_from_ledger(repr, ctx) {
            info!("[On-chain event] pool diff is defined: {:?}", pool_diff);
            let (plus_sign, diff) = pool_diff.lp_diff;
            info!("[On-chain event] pool plus sign: {}, diff {}", plus_sign, diff);
            if diff != 0 {
                info!("[On-chain event] Going to find acc");
                if let Some(account) =
                    find_lp_recv(pool_diff.lp_asset.into_token().unwrap(), pool_diff.pool_id, repr)
                {
                    info!("[On-chain event] Found account: {}", account.to_hex());
                    let account = account.payment_cred().unwrap().clone();
                    return Some(if plus_sign {
                        info!("[On-chain event] PositionEvent is deposit");
                        PositionEvent::Deposit(Deposit {
                            pool_id: pool_diff.pool_id,
                            account,
                            lp_mint: diff,
                            lp_supply: pool_diff.lp_supply,
                        })
                    } else {
                        info!("[On-chain event] PositionEvent is redeem");
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
        info!("[On-chain event] PositionEvent is none");
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
        info!("[On-chain event] Testing PoolDiff for {}", repr.hash.to_hex());
        let pool_in = repr.inputs.iter().find_map(|(input, maybe_utxo)| {
            maybe_utxo.as_ref().and_then(|u| AnyPool::try_from_ledger(u, ctx))
        });
        info!("[On-chain event] pool_in is defined: {:?}", pool_in.is_some());
        let pool_out = repr.outputs.iter().find_map(|u| AnyPool::try_from_ledger(u, ctx));
        info!("[On-chain event] pool_out is defined: {:?}", pool_out.is_some());
        if let (Some(pin), Some(pout)) = (pool_in, pool_out) {
            let (lp_in, lp_asset) = match pin {
                AnyPool::PureCFMM(p) => (p.liquidity.untag(), p.asset_lq.untag()),
                AnyPool::BalancedCFMM(p) => (p.liquidity.untag(), p.asset_lq.untag()),
                AnyPool::StableCFMM(p) => (p.liquidity.untag(), p.asset_lq.untag()),
            };
            info!("[On-chain event] lp_in: {:?}", lp_in);
            let lp_out = match pout {
                AnyPool::PureCFMM(p) => p.liquidity,
                AnyPool::BalancedCFMM(p) => p.liquidity,
                AnyPool::StableCFMM(p) => p.liquidity,
            }
            .untag();
            info!("[On-chain event] lp_out: {:?}", lp_out);
            let lp_diff = lp_out
                .checked_sub(lp_in)
                .map(|r| (true, r))
                .unwrap_or_else(|| (false, lp_in - lp_out));
            info!("[On-chain event] lp_diff: {:?}", lp_diff);
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

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq, Debug)]
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
        info!("[On-chain event] Testing output {}", output.address().to_hex());
        let test1 = output.value().multiasset.get(&pol, &tn.into()).is_some();
        let test2 = output
            .value()
            .multiasset
            .get(&pool_nft_pol, &pool_nft_tn.into())
            .is_none();
        info!("[On-chain event] Test 1: {}", test1);
        info!("[On-chain event] Test 2: {}", test2);
        if test1 && test2 {
            Some(output.address().clone())
        } else {
            None
        }
    })
}

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq, Debug)]
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

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq, Debug)]
pub struct Harvest {
    pub pool_id: PoolId,
    pub account: Credential,
    pub harvested_till: cml_chain::Slot,
}

impl Display for Harvest {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        let account = hex::encode(self.account.to_raw_bytes());
        write!(
            f,
            "Harvest(pool_id = {}, account = {}, harvested_till = {})",
            self.pool_id, account, self.harvested_till
        )
    }
}

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq, Debug)]
pub struct MultipleAccountsHarvest {
    pub accounts: Vec<Credential>,
    pub harvested_till: Slot,
}

impl<Cx> TryFromLedger<TransactionOutput, Cx> for MultipleAccountsHarvest
where
    Cx: Has<DeployedScriptInfo<{ ProtocolValidator::HarvestOrder as u8 }>>
        + Has<HarvestLimits>
        + Has<Slot>
        + Has<Vec<Ed25519KeyHash>>,
{
    fn try_from_ledger(repr: &TransactionOutput, ctx: &Cx) -> Option<Self> {
        let signers: Vec<Ed25519KeyHash> = ctx.get();
        let slot: Slot = ctx.get();
        let correct_lovelace_value = repr.value().coin as u64
            >= (signers.len() as u64 * ctx.select::<HarvestLimits>().minimal_lovelace_per_single_harvest);
        if test_address(repr.address(), ctx) && correct_lovelace_value {
            let accounts = signers.clone().into_iter().map(Credential::new_pub_key).collect();
            Some(MultipleAccountsHarvest {
                accounts,
                harvested_till: slot,
            })
        } else {
            None
        }
    }
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

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq, Debug)]
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
        info!(
            "[On-chain event] Testing farm created. Inputs len: {}",
            repr.inputs.len()
        );
        let farms_in_inputs: HashSet<_> =
            HashSet::from_iter(repr.inputs.iter().filter_map(|(i, maybe_utxo)| {
                maybe_utxo
                    .as_ref()
                    .and_then(|u| {
                        let oref = TimedOutputRef::new(OutputRef::from(i.clone()), Slot(repr.slot));
                        info!("[On-chain event] Going to test SmartFarmSnapshot parsing ++++");
                        let result = SmartFarmSnapshot::try_from_ledger(u, &ProvideTimedOref(ctx, oref));
                        info!("[On-chain event] SmartFarmSnapshot {}", result.is_some());
                        result
                    })
                    .map(|farm| {
                        info!("[On-chain event] SmartFarmSnapshot {:?}", farm);
                        farm.get().farm_id
                    })
            }));
        info!("[On-chain event] farms_in_inputs len: {}", farms_in_inputs.len());
        let farms_in_outputs = repr.outputs.iter().enumerate().filter_map(|(ix, utxo)| {
            let oref = TimedOutputRef::new(OutputRef::new(repr.hash, ix as u64), Slot(repr.slot));
            SmartFarmSnapshot::try_from_ledger(utxo, &ProvideTimedOref(ctx, oref))
        });
        let mut new_farms = farms_in_outputs.filter_map(|farm| {
            info!("[On-chain event] Farm in output {:?}", farm);
            if !farms_in_inputs.contains(&farm.get().farm_id) {
                Some(farm.unwrap())
            } else {
                None
            }
        });
        new_farms.next().map(|sm| {
            info!(
                "[On-chain event] SmartFarmSnapshot ({}) created for pool {}",
                sm.farm_id, sm.pool_id
            );
            FarmCreated {
                farm_id: sm.farm_id,
                pool_id: sm.pool_id,
            }
        })
    }
}

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq, Debug, Display)]
pub enum PollFactoryEvents {
    NewFactory(PollFactory),
    FactoryStateUpdate(PollFactoryUpdated),
}

impl<Cx> TryFromLedger<TxViewPartiallyResolved, Cx> for PollFactoryEvents
where
    Cx: Has<DeployedScriptInfo<{ ProtocolValidator::WpFactory as u8 }>> + Has<WPFactoryAuthPolicy>,
{
    fn try_from_ledger(repr: &TxViewPartiallyResolved, ctx: &Cx) -> Option<Self> {
        info!(
            "[On-chain event] PollFactoryEvents. Testing PollFactoryUpdated on {}. Resolved inputs {} ",
            repr.hash,
            repr.inputs.len()
        );
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
                let result = PollFactorySnapshot::try_from_ledger(utxo, &ProvideTimedOref(ctx, oref)).map(|snapshot| {
                    let mut inputs_factory = String::new();
                    factory_in_inputs.iter().for_each(|i| {
                        inputs_factory.push_str(format!(", {:?}", i).as_str());
                    });
                    info!("[On-chain event] PollFactoryEvents. Trying to determine kind of event, factory_in_inputs: {} (len {}), Current snapshot id {},  TxHash {}", inputs_factory, factory_in_inputs.len(), &snapshot.get().stable_id.to_hex(), repr.hash);
                    if factory_in_inputs.contains(&snapshot.get().stable_id) {
                        FactoryStateUpdate(
                            PollFactoryUpdated {
                                new_state: snapshot.get().clone(),
                            }
                        )
                    } else {
                        NewFactory(snapshot.get().clone())
                    }
                });
                if let Some(farm) = result {
                    info!("[On-chain event] PollFactoryEvents. Result of parsing smartFarmSnapshot {} for tx {}", farm, repr.hash);
                    Some(farm)
                } else {
                    None
                }
            })
            .next()
    }
}

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq, Debug, Display)]
pub struct PollFactoryUpdated {
    pub new_state: PollFactory,
}

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq, Debug, Display)]
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

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq, Debug, Display)]
#[display("FarmActivated ( pool_id = {}, slot = {})", pool_id, slot)]
pub struct FarmActivated {
    pub pool_id: PoolId,
    pub slot: Slot,
}

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq, Debug, Display)]
#[display("FarmDeactivated ( pool_id = {})", pool_id)]
pub struct FarmDeactivated {
    pub pool_id: PoolId,
}

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq, Debug, Display)]
pub enum PoolEvent {
    PoolCreated(PoolCreated),
}

impl PoolEvent {
    pub fn pool_id(&self) -> PoolId {
        match self {
            PoolEvent::PoolCreated(d) => d.pool_id,
        }
    }
}

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq, Debug, Display)]
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
        info!("[On-chain event] Testing PoolCreated for {}", repr.hash);
        let pool_in = repr.inputs.iter().find_map(|(input, maybe_utxo)| {
            maybe_utxo.as_ref().and_then(|u| AnyPool::try_from_ledger(u, ctx))
        });
        info!("[On-chain event] pool_in is defined: {:?}", pool_in.is_some());
        let pool_out = repr.outputs.iter().find_map(|u| AnyPool::try_from_ledger(u, ctx));
        info!("[On-chain event] pool_out is defined: {:?}", pool_in.is_some());
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
        info!("[On-chain event] PoolCreated is none");
        None
    }
}

#[cfg(test)]
mod tests {
    use crate::config::HarvestLimits;
    use crate::onchain::event::MultipleAccountsHarvest;
    use bloom_offchain_cardano::orders::grid::GridOrder;
    use cml_chain::transaction::TransactionOutput;
    use cml_core::serialization::Deserialize;
    use cml_crypto::Ed25519KeyHash;
    use type_equalities::IsEqual;
    use spectrum_offchain::domain::Has;
    use spectrum_offchain::ledger::TryFromLedger;
    use spectrum_offchain_cardano::deployment::ProtocolValidator::GridOrderNative;
    use spectrum_offchain_cardano::deployment::{
        DeployedScriptInfo, DeployedValidators, ProtocolScriptHashes,
    };
    use splash_dao_offchain::deployment::{
        DeployedValidators as DaoValidators, ProtocolDeployment as DaoDeployment, ProtocolTokens,
        ProtocolValidator,
    };
    use splash_dao_offchain::routines::Slot;

    struct Context {
        harvest_order: DeployedScriptInfo<{ ProtocolValidator::HarvestOrder as u8 }>,
        harvest_limits: HarvestLimits,
        slot: Slot,
        signers: Vec<Ed25519KeyHash>,
    }

    impl Has<DeployedScriptInfo<{ ProtocolValidator::HarvestOrder as u8 }>> for Context{
        fn select<U: IsEqual<DeployedScriptInfo<{ ProtocolValidator::HarvestOrder as u8 }>>>(&self) -> DeployedScriptInfo<{ ProtocolValidator::HarvestOrder as u8 }> {
            self.harvest_order
        }
    }

    impl Has<HarvestLimits> for Context {
        fn select<U: IsEqual<HarvestLimits>>(&self) -> HarvestLimits {
            self.harvest_limits
        }
    }

    impl Has<Slot> for Context {
        fn select<U: IsEqual<Slot>>(&self) -> Slot {
            self.slot
        }
    }

    impl Has<Vec<Ed25519KeyHash>> for Context {
        fn select<U: IsEqual<Vec<Ed25519KeyHash>>>(&self) -> Vec<Ed25519KeyHash> {
            self.signers.clone()
        }
    }

    #[test]
    fn try_read() {
        let raw_deployment = std::fs::read_to_string("/Users/aleksandr/IdeaProjects/spectrum-offchain-multiplatform/splash-lp-indexer/resources/preprod.dao.deployment.json").expect("Cannot load deployment file");
        let deployment: DaoValidators =
            serde_json::from_str(&raw_deployment).expect("Invalid deployment file");
        let ctx = Context {
            harvest_order: (&deployment.harvest_order).into(),
            harvest_limits: HarvestLimits {
                minimal_lovelace_per_single_harvest: 1000000,
            },
            slot: Slot(1234),
            signers: vec![],
        };
        let bearer = TransactionOutput::from_cbor_bytes(&*hex::decode(UTXO).unwrap()).unwrap();
        let ord = MultipleAccountsHarvest::try_from_ledger(&bearer, &ctx).unwrap();
        println!("Order: {:?}", ord);
    }

    const UTXO: &str = "a300581d707df8e5fd9f02bf01dac434bce324c2587d2a3f7d8de67aed993c5d53011a01312d00028201d8185840d8799f581c8d4be10d934b60a22f267699ea3f7ebdade1f8e535d1bd0ef7ce18b6581c79c7b50d79c32ea7b6bde64d4dfd5f595a725966bfdf1155385bddacff";
}
