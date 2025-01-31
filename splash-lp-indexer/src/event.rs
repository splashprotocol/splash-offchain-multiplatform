use crate::tx_view::TxViewPartiallyResolved;
use cml_chain::address::Address;
use cml_chain::certs::Credential;
use serde::{Deserialize, Serialize};
use spectrum_cardano_lib::transaction::TransactionOutputExtension;
use spectrum_cardano_lib::{AssetClass, Token};
use spectrum_offchain::domain::{Has, Stable};
use spectrum_offchain::ledger::TryFromLedger;
use spectrum_offchain_cardano::data::pool::{AnyPool, PoolValidation};
use spectrum_offchain_cardano::data::PoolId;
use spectrum_offchain_cardano::deployment::DeployedScriptInfo;
use spectrum_offchain_cardano::deployment::ProtocolValidator::{
    BalanceFnPoolV1, BalanceFnPoolV2, ConstFnPoolFeeSwitch, ConstFnPoolFeeSwitchBiDirFee,
    ConstFnPoolFeeSwitchV2, ConstFnPoolV1, ConstFnPoolV2, RoyaltyPoolV1, StableFnPoolT2T,
};

#[derive(Serialize, Deserialize)]
pub enum LpEvent {
    Deposit(Deposit),
    Redeem(Redeem),
}

impl LpEvent {
    pub fn account(&self) -> Credential {
        match self {
            LpEvent::Deposit(d) => d.account.clone(),
            LpEvent::Redeem(r) => r.account.clone(),
        }
    }
    pub fn pool_id(&self) -> PoolId {
        match self {
            LpEvent::Deposit(d) => d.pool_id,
            LpEvent::Redeem(r) => r.pool_id,
        }
    }
    pub fn lp_supply(&self) -> u64 {
        match self {
            LpEvent::Deposit(d) => d.lp_supply,
            LpEvent::Redeem(r) => r.lp_supply,
        }
    }
}

impl<Cx> TryFromLedger<TxViewPartiallyResolved, Cx> for LpEvent
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
                if let Some(account) = find_lp_recv(pool.lp_asset.into_token().unwrap(), repr) {
                    let account = account.payment_cred().unwrap().clone();
                    return Some(if plus_sign {
                        LpEvent::Deposit(Deposit {
                            pool_id: pool.pool_id,
                            account,
                            lp_mint: diff,
                            lp_supply: pool.lp_supply,
                        })
                    } else {
                        LpEvent::Redeem(Redeem {
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

#[derive(Serialize, Deserialize)]
pub struct Deposit {
    pub pool_id: PoolId,
    pub account: Credential,
    pub lp_mint: u64,
    pub lp_supply: u64,
}

fn find_lp_recv(Token(pol, tn): Token, tx: &TxViewPartiallyResolved) -> Option<Address> {
    tx.outputs.iter().find_map(|output| {
        output
            .value()
            .multiasset
            .get(&pol, &tn.into())
            .map(|_| output.address().clone())
    })
}

#[derive(Serialize, Deserialize)]
pub struct Redeem {
    pub pool_id: PoolId,
    pub account: Credential,
    pub lp_burned: u64,
    pub lp_supply: u64,
}

#[derive(Serialize, Deserialize)]
pub struct Harvest {
    pool_id: PoolId,
    account: Credential,
    rewards: Vec<(AssetClass, u64)>,
}

impl<Cx> TryFromLedger<TxViewPartiallyResolved, Cx> for Harvest {
    fn try_from_ledger(repr: &TxViewPartiallyResolved, ctx: &Cx) -> Option<Self> {
        todo!()
    }
}
