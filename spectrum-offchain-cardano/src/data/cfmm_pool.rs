use std::fmt::{Debug, Display};
use std::ops::{Div, Neg};

use cml_chain::address::Address;
use cml_chain::certs::StakeCredential;
use cml_chain::plutus::{ConstrPlutusData, PlutusData};
use cml_chain::transaction::TransactionOutput;
use cml_chain::utils::BigInteger;
use cml_core::serialization::{RawBytesEncoding, Serialize, ToBytes};
use num_traits::ToPrimitive;
use num_traits::{CheckedAdd, CheckedSub};
use type_equalities::IsEqual;
use void::Void;

use bloom_offchain::execution_engine::liquidity_book::core::Next;
use bloom_offchain::execution_engine::liquidity_book::market_maker::AvailableLiquidity;
use bloom_offchain::execution_engine::liquidity_book::market_maker::{
    AbsoluteReserves, MakerBehavior, MarketMaker, PoolQuality, SpotPrice,
};
use bloom_offchain::execution_engine::liquidity_book::side::{OnSide, Side};
use bloom_offchain::execution_engine::liquidity_book::types::AbsolutePrice;
use spectrum_cardano_lib::address::AddressExtension;
use spectrum_cardano_lib::ex_units::ExUnits;
use spectrum_cardano_lib::plutus_data::{
    ConstrPlutusDataExtension, DatumExtension, IntoPlutusData, PlutusDataExtension,
};
use spectrum_cardano_lib::transaction::TransactionOutputExtension;
use spectrum_cardano_lib::types::TryFromPData;
use spectrum_cardano_lib::value::ValueExtension;
use spectrum_cardano_lib::{NetworkId, TaggedAmount, TaggedAssetClass};
use spectrum_offchain::domain::{Has, Stable};
use spectrum_offchain::ledger::{IntoLedger, TryFromLedger};

use crate::data::cfmm_pool::classic_pool::ClassicPool;
use crate::data::cfmm_pool::fee_switch_pool::{unsafe_update_pd_fee_switch, FeeSwitchPool};
use crate::data::cfmm_pool::royalty_pool::{
    unsafe_update_pd_royalty, unsafe_update_pd_royalty_v2, RoyaltyPool, RoyaltyPoolVer,
};
use crate::data::dao_request::{DAOContext, OnChainDAOActionRequest};
use crate::data::deposit::ClassicalOnChainDeposit;
use crate::data::operation_output::{
    DaoActionResult, DepositOutput, OperationResultBlueprint, RedeemOutput, RoyaltyWithdrawOutput,
};
use crate::data::order::{Base, PoolNft, Quote};
use crate::data::pair::order_canonical;
use crate::data::pool::{
    ApplyOrder, ApplyOrderError, ImmutablePoolUtxo, Lq, PoolAssetMapping, PoolValidation, Rx, Ry,
};
use crate::data::redeem::ClassicalOnChainRedeem;
use crate::data::royalty_withdraw_request::{OnChainRoyaltyWithdraw, RoyaltyWithdrawContext};
use crate::data::PoolId;
use crate::deployment::ProtocolValidator::{
    BalanceFnPoolDeposit, BalanceFnPoolRedeem, ConstFnFeeSwitchPoolDeposit, ConstFnFeeSwitchPoolRedeem,
    ConstFnPoolDeposit, ConstFnPoolFeeSwitch, ConstFnPoolFeeSwitchBiDirFee, ConstFnPoolFeeSwitchV2,
    ConstFnPoolRedeem, ConstFnPoolV1, ConstFnPoolV2, RoyaltyPoolDAOV1, RoyaltyPoolDAOV1Request,
    RoyaltyPoolRoyaltyWithdraw, RoyaltyPoolRoyaltyWithdrawLedgerFixed, RoyaltyPoolRoyaltyWithdrawV2,
    RoyaltyPoolV1, RoyaltyPoolV1Deposit, RoyaltyPoolV1LedgerFixed, RoyaltyPoolV1Redeem,
    RoyaltyPoolV1RoyaltyWithdrawRequest, RoyaltyPoolV2, RoyaltyPoolV2DAO, RoyaltyPoolV2Deposit,
    RoyaltyPoolV2Redeem, RoyaltyPoolV2RoyaltyWithdrawRequest, StableFnPoolT2TDeposit, StableFnPoolT2TRedeem,
};
use crate::deployment::{DeployedScriptInfo, DeployedValidator, DeployedValidatorErased, RequiresValidator};
use crate::fees::FeeExtension;

pub mod classic_pool;
pub mod fee_switch_pool;
pub mod royalty_pool;

pub struct LegacyCFMMPoolConfig {
    pub pool_nft: TaggedAssetClass<PoolNft>,
    pub asset_x: TaggedAssetClass<Rx>,
    pub asset_y: TaggedAssetClass<Ry>,
    pub asset_lq: TaggedAssetClass<Lq>,
    pub lp_fee_num: u64,
    pub lq_lower_bound: TaggedAmount<Rx>,
}

impl TryFromPData for LegacyCFMMPoolConfig {
    fn try_from_pd(data: PlutusData) -> Option<Self> {
        let mut cpd = data.into_constr_pd()?;
        let pool_nft = TaggedAssetClass::try_from_pd(cpd.take_field(0)?)?;
        let asset_x = TaggedAssetClass::try_from_pd(cpd.take_field(1)?)?;
        let asset_y = TaggedAssetClass::try_from_pd(cpd.take_field(2)?)?;
        let asset_lq = TaggedAssetClass::try_from_pd(cpd.take_field(3)?)?;
        let lp_fee_num = cpd.take_field(4)?.into_u64()?;
        let lq_lower_bound = TaggedAmount::new(cpd.take_field(6).and_then(|pd| pd.into_u64()).unwrap_or(0));
        Some(Self {
            pool_nft,
            asset_x,
            asset_y,
            asset_lq,
            lp_fee_num,
            lq_lower_bound,
        })
    }
}

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub enum ClassicPoolVer {
    V1,
    V2,
}

impl ClassicPoolVer {
    pub fn try_from_address<Ctx>(pool_addr: &Address, ctx: &Ctx) -> Option<ClassicPoolVer>
    where
        Ctx: Has<DeployedScriptInfo<{ ConstFnPoolV1 as u8 }>>
            + Has<DeployedScriptInfo<{ ConstFnPoolV2 as u8 }>>,
    {
        let maybe_hash = pool_addr.payment_cred().and_then(|c| match c {
            StakeCredential::PubKey { .. } => None,
            StakeCredential::Script { hash, .. } => Some(hash),
        });
        if let Some(this_hash) = maybe_hash {
            if ctx
                .select::<DeployedScriptInfo<{ ConstFnPoolV1 as u8 }>>()
                .script_hash
                == *this_hash
            {
                return Some(ClassicPoolVer::V1);
            } else if ctx
                .select::<DeployedScriptInfo<{ ConstFnPoolV2 as u8 }>>()
                .script_hash
                == *this_hash
            {
                return Some(ClassicPoolVer::V2);
            }
        };
        None
    }
}

pub struct AbsolutePoolReserves<'a> {
    pub reserves_x: &'a mut TaggedAmount<Rx>,
    pub reserves_y: &'a mut TaggedAmount<Ry>,
    pub reserves_lq: &'a mut TaggedAmount<Lq>,
}

#[derive(Debug, Copy, Clone, Eq, PartialEq, derive_more::Display)]
pub enum ConstFnPool {
    Classic(ClassicPool),
    FeeSwitch(FeeSwitchPool),
    Royalty(RoyaltyPool),
}

impl ConstFnPool {
    pub fn asset_x(&self) -> TaggedAssetClass<Rx> {
        match self {
            ConstFnPool::Classic(pool) => pool.asset_x,
            ConstFnPool::FeeSwitch(pool) => pool.asset_x,
            ConstFnPool::Royalty(pool) => pool.asset_x,
        }
    }

    pub fn asset_y(&self) -> TaggedAssetClass<Ry> {
        match self {
            ConstFnPool::Classic(pool) => pool.asset_y,
            ConstFnPool::FeeSwitch(pool) => pool.asset_y,
            ConstFnPool::Royalty(pool) => pool.asset_y,
        }
    }

    pub fn asset_mapping(&self, side: Side) -> PoolAssetMapping {
        let x = self.asset_x().untag();
        let y = self.asset_y().untag();
        let [base, _] = order_canonical(x, y);
        if base == x {
            match side {
                Side::Bid => PoolAssetMapping {
                    asset_to_deduct_from: x,
                    asset_to_add_to: y,
                },
                Side::Ask => PoolAssetMapping {
                    asset_to_deduct_from: y,
                    asset_to_add_to: x,
                },
            }
        } else {
            match side {
                Side::Bid => PoolAssetMapping {
                    asset_to_deduct_from: y,
                    asset_to_add_to: x,
                },
                Side::Ask => PoolAssetMapping {
                    asset_to_deduct_from: x,
                    asset_to_add_to: y,
                },
            }
        }
    }

    pub fn unsafe_datum_update(&self, raw_datum: &mut PlutusData) {
        match self {
            ConstFnPool::FeeSwitch(fee_switch) => unsafe_update_pd_fee_switch(
                raw_datum,
                fee_switch.treasury_x.untag(),
                fee_switch.treasury_y.untag(),
            ),
            ConstFnPool::Royalty(royalty_pool) => match royalty_pool.ver {
                RoyaltyPoolVer::V1 | RoyaltyPoolVer::V1LedgerFixed => unsafe_update_pd_royalty(
                    raw_datum,
                    *royalty_pool.lp_fee.numer(),
                    *royalty_pool.treasury_fee.numer(),
                    *royalty_pool.first_royalty_fee.numer(),
                    royalty_pool.treasury_x.untag(),
                    royalty_pool.treasury_y.untag(),
                    royalty_pool.first_royalty_x.untag(),
                    royalty_pool.first_royalty_y.untag(),
                    royalty_pool.treasury_address,
                    royalty_pool.admin_address,
                    royalty_pool.nonce,
                ),
                RoyaltyPoolVer::V2 => unsafe_update_pd_royalty_v2(
                    raw_datum,
                    *royalty_pool.lp_fee.numer(),
                    *royalty_pool.treasury_fee.numer(),
                    *royalty_pool.first_royalty_fee.numer(),
                    *royalty_pool.second_royalty_fee.numer(),
                    royalty_pool.treasury_x.untag(),
                    royalty_pool.treasury_y.untag(),
                    royalty_pool.first_royalty_x.untag(),
                    royalty_pool.first_royalty_y.untag(),
                    royalty_pool.second_royalty_x.untag(),
                    royalty_pool.second_royalty_y.untag(),
                    royalty_pool.treasury_address,
                    royalty_pool.admin_address,
                    royalty_pool.nonce,
                ),
            },
            _ => {}
        }
    }
}

pub struct CFMMPoolRedeemer {
    pub pool_input_index: u64,
    pub action: crate::data::pool::CFMMPoolAction,
}

impl CFMMPoolRedeemer {
    pub fn to_plutus_data(self) -> PlutusData {
        let action_pd = self.action.to_plutus_data();
        let self_ix_pd = PlutusData::Integer(BigInteger::from(self.pool_input_index));
        PlutusData::ConstrPlutusData(ConstrPlutusData::new(0, vec![action_pd, self_ix_pd]))
    }
}

pub trait AMMOps {
    fn output_amount(
        &self,
        base_asset: TaggedAssetClass<Base>,
        base_amount: TaggedAmount<Base>,
    ) -> TaggedAmount<Quote>;

    fn reward_lp(
        &self,
        in_x_amount: u64,
        in_y_amount: u64,
    ) -> Option<(TaggedAmount<Lq>, TaggedAmount<Rx>, TaggedAmount<Ry>)>;

    fn shares_amount(&self, burned_lq: TaggedAmount<Lq>) -> Option<(TaggedAmount<Rx>, TaggedAmount<Ry>)>;
}

impl AMMOps for ConstFnPool {
    fn output_amount(
        &self,
        base_asset: TaggedAssetClass<Base>,
        base_amount: TaggedAmount<Base>,
    ) -> TaggedAmount<Quote> {
        match self {
            ConstFnPool::Classic(const_pool) => const_pool.output_amount(base_asset, base_amount),
            ConstFnPool::FeeSwitch(fee_switchPool) => fee_switchPool.output_amount(base_asset, base_amount),
            ConstFnPool::Royalty(royalty_pool) => royalty_pool.output_amount(base_asset, base_amount),
        }
    }

    fn reward_lp(
        &self,
        in_x_amount: u64,
        in_y_amount: u64,
    ) -> Option<(TaggedAmount<Lq>, TaggedAmount<Rx>, TaggedAmount<Ry>)> {
        match self {
            ConstFnPool::Classic(const_pool) => const_pool.reward_lp(in_x_amount, in_y_amount),
            ConstFnPool::FeeSwitch(fee_switchPool) => fee_switchPool.reward_lp(in_x_amount, in_y_amount),
            ConstFnPool::Royalty(royalty_pool) => royalty_pool.reward_lp(in_x_amount, in_y_amount),
        }
    }

    fn shares_amount(&self, burned_lq: TaggedAmount<Lq>) -> Option<(TaggedAmount<Rx>, TaggedAmount<Ry>)> {
        match self {
            ConstFnPool::Classic(const_pool) => const_pool.shares_amount(burned_lq),
            ConstFnPool::FeeSwitch(fee_switchPool) => fee_switchPool.shares_amount(burned_lq),
            ConstFnPool::Royalty(royalty_pool) => royalty_pool.shares_amount(burned_lq),
        }
    }
}

impl<Ctx> RequiresValidator<Ctx> for ConstFnPool
where
    Ctx: Has<DeployedValidator<{ ConstFnPoolV1 as u8 }>>
        + Has<DeployedValidator<{ ConstFnPoolV2 as u8 }>>
        + Has<DeployedValidator<{ ConstFnPoolFeeSwitch as u8 }>>
        + Has<DeployedValidator<{ ConstFnPoolFeeSwitchV2 as u8 }>>
        + Has<DeployedValidator<{ ConstFnPoolFeeSwitchBiDirFee as u8 }>>
        + Has<DeployedValidator<{ RoyaltyPoolV1 as u8 }>>
        + Has<DeployedValidator<{ RoyaltyPoolV1LedgerFixed as u8 }>>
        + Has<DeployedValidator<{ RoyaltyPoolV2 as u8 }>>,
{
    fn get_validator(&self, ctx: &Ctx) -> DeployedValidatorErased {
        match self {
            ConstFnPool::Classic(const_pool) => const_pool.get_validator(ctx),
            ConstFnPool::FeeSwitch(fee_switchPool) => fee_switchPool.get_validator(ctx),
            ConstFnPool::Royalty(royalty_pool) => royalty_pool.get_validator(ctx),
        }
    }
}

impl MakerBehavior for ClassicPool {
    fn swap(mut self, input: OnSide<u64>) -> Next<Self, Void> {
        let x = self.asset_x.untag();
        let y = self.asset_y.untag();
        let [base, quote] = order_canonical(x, y);
        let output = match input {
            OnSide::Bid(input) => self
                .output_amount(TaggedAssetClass::new(quote), TaggedAmount::new(input))
                .untag(),
            OnSide::Ask(input) => self
                .output_amount(TaggedAssetClass::new(base), TaggedAmount::new(input))
                .untag(),
        };
        let (base_reserves, quote_reserves) = if x == base {
            (self.reserves_x.as_mut(), self.reserves_y.as_mut())
        } else {
            (self.reserves_y.as_mut(), self.reserves_x.as_mut())
        };
        match input {
            OnSide::Bid(input) => {
                // A user bid means that they wish to buy the base asset for the quote asset, hence
                // pool reserves of base decreases while reserves of quote increase.
                *quote_reserves += input;
                *base_reserves -= output;
            }
            OnSide::Ask(input) => {
                // User ask is the opposite; sell the base asset for the quote asset.
                *base_reserves += input;
                *quote_reserves -= output;
            }
        }
        Next::Succ(self)
    }
}

impl MakerBehavior for ConstFnPool {
    fn swap(mut self, input: OnSide<u64>) -> Next<Self, Void> {
        match self {
            ConstFnPool::Classic(classic) => classic
                .swap(input)
                .map_succ(|const_fn_pool| ConstFnPool::Classic(const_fn_pool)),
            ConstFnPool::FeeSwitch(fee_switch) => fee_switch
                .swap(input)
                .map_succ(|fee_switch_pool| ConstFnPool::FeeSwitch(fee_switch_pool)),
            ConstFnPool::Royalty(royalty_pool) => royalty_pool
                .swap(input)
                .map_succ(|royalty_pool| ConstFnPool::Royalty(royalty_pool)),
        }
    }
}

impl MarketMaker for ConstFnPool {
    type U = ExUnits;

    fn static_price(&self) -> SpotPrice {
        match self {
            ConstFnPool::Classic(classic) => classic.static_price(),
            ConstFnPool::FeeSwitch(fee_switch) => fee_switch.static_price(),
            ConstFnPool::Royalty(royalty) => royalty.static_price(),
        }
    }

    fn real_price(&self, input: OnSide<u64>) -> Option<AbsolutePrice> {
        match self {
            ConstFnPool::Classic(classic) => classic.real_price(input),
            ConstFnPool::FeeSwitch(fee_switch) => fee_switch.real_price(input),
            ConstFnPool::Royalty(royalty) => royalty.real_price(input),
        }
    }

    fn quality(&self) -> PoolQuality {
        match self {
            ConstFnPool::Classic(classic) => classic.quality(),
            ConstFnPool::FeeSwitch(fee_switch) => fee_switch.quality(),
            ConstFnPool::Royalty(royalty) => royalty.quality(),
        }
    }

    fn marginal_cost_hint(&self) -> ExUnits {
        match self {
            ConstFnPool::Classic(classic) => classic.marginal_cost_hint(),
            ConstFnPool::FeeSwitch(fee_switch) => fee_switch.marginal_cost_hint(),
            ConstFnPool::Royalty(royalty) => royalty.marginal_cost_hint(),
        }
    }

    fn is_active(&self) -> bool {
        match self {
            ConstFnPool::Classic(classic) => classic.is_active(),
            ConstFnPool::FeeSwitch(fee_switch) => fee_switch.is_active(),
            ConstFnPool::Royalty(royalty) => royalty.is_active(),
        }
    }

    fn liquidity(&self) -> AbsoluteReserves {
        match self {
            ConstFnPool::Classic(classic) => classic.liquidity(),
            ConstFnPool::FeeSwitch(fee_switch) => fee_switch.liquidity(),
            ConstFnPool::Royalty(royalty) => royalty.liquidity(),
        }
    }

    fn available_liquidity_on_side(&self, worst_price: OnSide<AbsolutePrice>) -> Option<AvailableLiquidity> {
        match self {
            ConstFnPool::Classic(classic) => classic.available_liquidity_on_side(worst_price),
            ConstFnPool::FeeSwitch(fee_switch) => fee_switch.available_liquidity_on_side(worst_price),
            ConstFnPool::Royalty(royalty) => royalty.available_liquidity_on_side(worst_price),
        }
    }

    fn estimated_trade(&self, input: OnSide<u64>) -> Option<AvailableLiquidity> {
        match self {
            ConstFnPool::Classic(classic) => classic.estimated_trade(input),
            ConstFnPool::FeeSwitch(fee_switch) => fee_switch.estimated_trade(input),
            ConstFnPool::Royalty(royalty) => royalty.estimated_trade(input),
        }
    }
}

impl Stable for ClassicPool {
    type StableId = PoolId;

    fn stable_id(&self) -> Self::StableId {
        self.id
    }

    fn is_quasi_permanent(&self) -> bool {
        true
    }
}

impl Stable for ConstFnPool {
    type StableId = PoolId;
    fn stable_id(&self) -> Self::StableId {
        match self {
            ConstFnPool::Classic(const_pool) => const_pool.stable_id(),
            ConstFnPool::FeeSwitch(fee_switch) => fee_switch.stable_id(),
            ConstFnPool::Royalty(royalty_pool) => royalty_pool.stable_id(),
        }
    }
    fn is_quasi_permanent(&self) -> bool {
        match self {
            ConstFnPool::Classic(const_pool) => const_pool.is_quasi_permanent(),
            ConstFnPool::FeeSwitch(fee_switch) => fee_switch.is_quasi_permanent(),
            ConstFnPool::Royalty(royalty_pool) => royalty_pool.is_quasi_permanent(),
        }
    }
}

impl<Ctx> TryFromLedger<TransactionOutput, Ctx> for ConstFnPool
where
    Ctx: Has<DeployedScriptInfo<{ ConstFnPoolV1 as u8 }>>
        + Has<DeployedScriptInfo<{ ConstFnPoolV2 as u8 }>>
        + Has<DeployedScriptInfo<{ ConstFnPoolFeeSwitch as u8 }>>
        + Has<DeployedScriptInfo<{ ConstFnPoolFeeSwitchV2 as u8 }>>
        + Has<DeployedScriptInfo<{ ConstFnPoolFeeSwitchBiDirFee as u8 }>>
        + Has<DeployedScriptInfo<{ RoyaltyPoolV1 as u8 }>>
        + Has<DeployedScriptInfo<{ RoyaltyPoolV1LedgerFixed as u8 }>>
        + Has<DeployedScriptInfo<{ RoyaltyPoolV2 as u8 }>>
        + Has<PoolValidation>,
{
    fn try_from_ledger(repr: &TransactionOutput, ctx: &Ctx) -> Option<Self> {
        ClassicPool::try_from_ledger(repr, ctx)
            .map(ConstFnPool::Classic)
            .or_else(|| FeeSwitchPool::try_from_ledger(repr, ctx).map(ConstFnPool::FeeSwitch))
            .or_else(|| RoyaltyPool::try_from_ledger(repr, ctx).map(ConstFnPool::Royalty))
    }
}

impl IntoLedger<TransactionOutput, ImmutablePoolUtxo> for ConstFnPool {
    fn into_ledger(self, mut immut_pool: ImmutablePoolUtxo) -> TransactionOutput {
        match self {
            ConstFnPool::Classic(const_pool) => const_pool.into_ledger(immut_pool),
            ConstFnPool::FeeSwitch(fee_switch) => fee_switch.into_ledger(immut_pool),
            ConstFnPool::Royalty(royalty_pool) => royalty_pool.into_ledger(immut_pool),
        }
    }
}

impl<Ctx> ApplyOrder<ClassicalOnChainDeposit, Ctx> for ConstFnPool
where
    Ctx: Has<DeployedValidator<{ ConstFnFeeSwitchPoolDeposit as u8 }>>
        + Has<DeployedValidator<{ ConstFnPoolDeposit as u8 }>>
        + Has<DeployedValidator<{ BalanceFnPoolDeposit as u8 }>>
        + Has<DeployedValidator<{ StableFnPoolT2TDeposit as u8 }>>
        + Has<DeployedValidator<{ RoyaltyPoolV1Deposit as u8 }>>
        + Has<DeployedValidator<{ RoyaltyPoolV2Deposit as u8 }>>,
{
    type Result = DepositOutput;

    fn apply_order(
        mut self,
        deposit: ClassicalOnChainDeposit,
        ctx: Ctx,
    ) -> Result<(Self, OperationResultBlueprint<DepositOutput>), ApplyOrderError<ClassicalOnChainDeposit>>
    {
        let order = deposit.order;
        let validator = deposit.get_validator(&ctx);
        let net_x = if order.token_x.is_native() {
            order
                .token_x_amount
                .untag()
                .checked_sub(order.ex_fee)
                .and_then(|result| result.checked_sub(order.collateral_ada))
                .ok_or(ApplyOrderError::incompatible(deposit.clone()))?
        } else {
            order.token_x_amount.untag()
        };

        let net_y = if order.token_y.is_native() {
            order
                .token_y_amount
                .untag()
                .checked_sub(order.ex_fee)
                .and_then(|result| result.checked_sub(order.collateral_ada))
                .ok_or(ApplyOrderError::incompatible(deposit.clone()))?
        } else {
            order.token_y_amount.untag()
        };

        let reward_lp_opt = self.reward_lp(net_x, net_y);

        let reserves = match self {
            ConstFnPool::Classic(ref mut const_pool) => AbsolutePoolReserves {
                reserves_x: &mut const_pool.reserves_x,
                reserves_y: &mut const_pool.reserves_y,
                reserves_lq: &mut const_pool.liquidity,
            },
            ConstFnPool::FeeSwitch(ref mut fee_switch) => AbsolutePoolReserves {
                reserves_x: &mut fee_switch.reserves_x,
                reserves_y: &mut fee_switch.reserves_y,
                reserves_lq: &mut fee_switch.liquidity,
            },
            ConstFnPool::Royalty(ref mut royalty) => AbsolutePoolReserves {
                reserves_x: &mut royalty.reserves_x,
                reserves_y: &mut royalty.reserves_y,
                reserves_lq: &mut royalty.liquidity,
            },
        };

        if let Some((unlocked_lq, change_x, change_y)) = reward_lp_opt {
            *reserves.reserves_x = reserves
                .reserves_x
                .checked_add(&TaggedAmount::new(net_x))
                .and_then(|result| result.checked_sub(&change_x))
                .ok_or(ApplyOrderError::incompatible(deposit.clone()))?;
            *reserves.reserves_y = reserves
                .reserves_y
                .checked_add(&TaggedAmount::new(net_y))
                .and_then(|result| result.checked_sub(&change_y))
                .ok_or(ApplyOrderError::incompatible(deposit.clone()))?;
            *reserves.reserves_lq = reserves
                .reserves_lq
                .checked_add(&unlocked_lq)
                .ok_or(ApplyOrderError::incompatible(deposit.clone()))?;

            let deposit_output = DepositOutput {
                token_x_asset: order.token_x,
                token_x_charge_amount: change_x,
                token_y_asset: order.token_y,
                token_y_charge_amount: change_y,
                token_lq_asset: order.token_lq,
                token_lq_amount: unlocked_lq,
                ada_residue: order.collateral_ada,
                redeemer_pkh: order.reward_pkh,
                redeemer_stake_pkh: order.reward_stake_pkh,
            };

            Ok((
                self,
                OperationResultBlueprint::single_output(deposit_output, validator),
            ))
        } else {
            Err(ApplyOrderError::incompatible(deposit))
        }
    }
}

impl<Ctx> ApplyOrder<ClassicalOnChainRedeem, Ctx> for ConstFnPool
where
    Ctx: Has<DeployedValidator<{ ConstFnFeeSwitchPoolRedeem as u8 }>>
        + Has<DeployedValidator<{ ConstFnPoolRedeem as u8 }>>
        + Has<DeployedValidator<{ BalanceFnPoolRedeem as u8 }>>
        + Has<DeployedValidator<{ StableFnPoolT2TRedeem as u8 }>>
        + Has<DeployedValidator<{ RoyaltyPoolV1Redeem as u8 }>>
        + Has<DeployedValidator<{ RoyaltyPoolV2Redeem as u8 }>>,
{
    type Result = RedeemOutput;

    fn apply_order(
        mut self,
        redeem: ClassicalOnChainRedeem,
        ctx: Ctx,
    ) -> Result<(Self, OperationResultBlueprint<RedeemOutput>), ApplyOrderError<ClassicalOnChainRedeem>> {
        let order = redeem.order;
        let validator = redeem.get_validator(&ctx);
        match self.shares_amount(order.token_lq_amount) {
            Some((x_amount, y_amount)) => {
                let reserves = match self {
                    ConstFnPool::Classic(ref mut const_pool) => AbsolutePoolReserves {
                        reserves_x: &mut const_pool.reserves_x,
                        reserves_y: &mut const_pool.reserves_y,
                        reserves_lq: &mut const_pool.liquidity,
                    },
                    ConstFnPool::FeeSwitch(ref mut fee_switch) => AbsolutePoolReserves {
                        reserves_x: &mut fee_switch.reserves_x,
                        reserves_y: &mut fee_switch.reserves_y,
                        reserves_lq: &mut fee_switch.liquidity,
                    },
                    ConstFnPool::Royalty(ref mut royalty) => AbsolutePoolReserves {
                        reserves_x: &mut royalty.reserves_x,
                        reserves_y: &mut royalty.reserves_y,
                        reserves_lq: &mut royalty.liquidity,
                    },
                };

                *reserves.reserves_x = reserves
                    .reserves_x
                    .checked_sub(&x_amount)
                    .ok_or(ApplyOrderError::incompatible(redeem.clone()))?;

                *reserves.reserves_y = reserves
                    .reserves_y
                    .checked_sub(&y_amount)
                    .ok_or(ApplyOrderError::incompatible(redeem.clone()))?;

                *reserves.reserves_lq = reserves
                    .reserves_lq
                    .checked_sub(&order.token_lq_amount)
                    .ok_or(ApplyOrderError::incompatible(redeem))?;

                let redeem_output = RedeemOutput {
                    token_x_asset: order.token_x,
                    token_x_amount: x_amount,
                    token_y_asset: order.token_y,
                    token_y_amount: y_amount,
                    ada_residue: order.collateral_ada,
                    redeemer_pkh: order.reward_pkh,
                    redeemer_stake_pkh: order.reward_stake_pkh,
                };

                Ok((
                    self,
                    OperationResultBlueprint::single_output(redeem_output, validator),
                ))
            }
            None => Err(ApplyOrderError::incompatible(redeem)),
        }
    }
}

impl<Ctx> ApplyOrder<OnChainRoyaltyWithdraw, Ctx> for ConstFnPool
where
    Ctx: Has<DeployedValidator<{ RoyaltyPoolRoyaltyWithdraw as u8 }>>
        + Has<DeployedValidator<{ RoyaltyPoolRoyaltyWithdrawLedgerFixed as u8 }>>
        + Has<DeployedValidator<{ RoyaltyPoolRoyaltyWithdrawV2 as u8 }>>
        + Has<DeployedValidator<{ RoyaltyPoolV1RoyaltyWithdrawRequest as u8 }>>
        + Has<DeployedValidator<{ RoyaltyPoolV2RoyaltyWithdrawRequest as u8 }>>
        + Has<RoyaltyWithdrawContext>,
{
    type Result = RoyaltyWithdrawOutput;

    fn apply_order(
        mut self,
        royalty_withdraw: OnChainRoyaltyWithdraw,
        ctx: Ctx,
    ) -> Result<
        (Self, OperationResultBlueprint<RoyaltyWithdrawOutput>),
        ApplyOrderError<OnChainRoyaltyWithdraw>,
    > {
        match self {
            ConstFnPool::Royalty(royalty_pool) => royalty_pool
                .apply_order(royalty_withdraw, ctx)
                .map(|(pool, result)| (ConstFnPool::Royalty(pool), result)),
            _ => Err(ApplyOrderError::incompatible(royalty_withdraw)),
        }
    }
}

impl<Ctx> ApplyOrder<OnChainDAOActionRequest, Ctx> for ConstFnPool
where
    Ctx: Has<DeployedValidator<{ RoyaltyPoolDAOV1 as u8 }>>
        + Has<DeployedValidator<{ RoyaltyPoolV2DAO as u8 }>>
        + Has<DeployedValidator<{ RoyaltyPoolDAOV1Request as u8 }>>
        + Has<DeployedValidator<{ RoyaltyPoolV1 as u8 }>>
        + Has<DeployedValidator<{ RoyaltyPoolV2 as u8 }>>
        + Has<DAOContext>
        + Has<NetworkId>,
{
    type Result = DaoActionResult;

    fn apply_order(
        mut self,
        dao_request: OnChainDAOActionRequest,
        ctx: Ctx,
    ) -> Result<(Self, OperationResultBlueprint<DaoActionResult>), ApplyOrderError<OnChainDAOActionRequest>>
    {
        match self {
            ConstFnPool::Royalty(r_pool) => r_pool
                .apply_order(dao_request, ctx)
                .map(|(pool, result)| (ConstFnPool::Royalty(pool), result)),
            _ => Err(ApplyOrderError::incompatible(dao_request)),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::convert::identity;

    use cml_chain::transaction::TransactionOutput;
    use cml_core::serialization::Deserialize;
    use cml_crypto::ScriptHash;
    use num_rational::Ratio;
    use num_traits::ToPrimitive;
    use type_equalities::IsEqual;

    use bloom_offchain::execution_engine::liquidity_book::core::{Excess, MakeInProgress, Next, Trans};
    use bloom_offchain::execution_engine::liquidity_book::market_maker::{
        AvailableLiquidity, MakerBehavior, MarketMaker,
    };
    use bloom_offchain::execution_engine::liquidity_book::side::OnSide::{Ask, Bid};
    use bloom_offchain::execution_engine::liquidity_book::side::{OnSide, Side};
    use bloom_offchain::execution_engine::liquidity_book::types::AbsolutePrice;
    use spectrum_cardano_lib::ex_units::ExUnits;
    use spectrum_cardano_lib::{AssetClass, AssetName, TaggedAmount, TaggedAssetClass, Token};
    use spectrum_offchain::domain::Has;
    use spectrum_offchain::ledger::TryFromLedger;

    use crate::data::cfmm_pool::fee_switch_pool::{FeeSwitchPool, FeeSwitchPoolVer};
    use crate::data::cfmm_pool::ConstFnPool;
    use crate::data::pool::PoolValidation;
    use crate::data::PoolId;
    use crate::deployment::ProtocolValidator::{
        ConstFnPoolFeeSwitch, ConstFnPoolFeeSwitchBiDirFee, ConstFnPoolFeeSwitchV2, ConstFnPoolV1,
        ConstFnPoolV2, RoyaltyPoolV1, RoyaltyPoolV1LedgerFixed, RoyaltyPoolV2,
    };
    use crate::deployment::{DeployedScriptInfo, DeployedValidators, ProtocolScriptHashes};

    fn gen_ada_token_pool(
        reserves_x: u64,
        reserves_y: u64,
        liquidity: u64,
        lp_fee_x: u64,
        lp_fee_y: u64,
        treasury_fee: u64,
        treasury_x: u64,
        treasury_y: u64,
        native_ind: u64,
    ) -> FeeSwitchPool {
        FeeSwitchPool {
            id: PoolId::from(Token(
                ScriptHash::from([
                    162, 206, 112, 95, 150, 240, 52, 167, 61, 102, 158, 92, 11, 47, 25, 41, 48, 224, 188,
                    211, 138, 203, 127, 107, 246, 89, 115, 157,
                ]),
                AssetName::from((
                    3,
                    [
                        110, 102, 116, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
                        0, 0, 0, 0, 0, 0,
                    ],
                )),
            )),
            reserves_x: TaggedAmount::new(reserves_x),
            reserves_y: TaggedAmount::new(reserves_y),
            liquidity: TaggedAmount::new(liquidity),
            asset_x: if native_ind == 0 {
                TaggedAssetClass::new(AssetClass::Native)
            } else {
                TaggedAssetClass::new(AssetClass::Token(Token(
                    ScriptHash::from([
                        75, 52, 89, 253, 24, 161, 219, 171, 226, 7, 205, 25, 201, 149, 26, 159, 172, 159, 92,
                        15, 156, 56, 78, 61, 151, 239, 186, 38,
                    ]),
                    AssetName::from((
                        5,
                        [
                            116, 101, 115, 116, 67, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
                            0, 0, 0, 0, 0, 0, 0, 0,
                        ],
                    )),
                )))
            },
            asset_y: if native_ind == 1 {
                TaggedAssetClass::new(AssetClass::Native)
            } else {
                TaggedAssetClass::new(AssetClass::Token(Token(
                    ScriptHash::from([
                        75, 52, 89, 253, 24, 161, 219, 171, 226, 7, 205, 25, 201, 149, 26, 159, 172, 159, 92,
                        15, 156, 56, 78, 61, 151, 239, 186, 38,
                    ]),
                    AssetName::from((
                        5,
                        [
                            116, 101, 115, 116, 67, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
                            0, 0, 0, 0, 0, 0, 0, 0,
                        ],
                    )),
                )))
            },
            asset_lq: TaggedAssetClass::new(AssetClass::Token(Token(
                ScriptHash::from([
                    114, 191, 27, 172, 195, 20, 1, 41, 111, 158, 228, 210, 254, 123, 132, 165, 36, 56, 38,
                    251, 3, 233, 206, 25, 51, 218, 254, 192,
                ]),
                AssetName::from((
                    2,
                    [
                        108, 113, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
                        0, 0, 0, 0, 0,
                    ],
                )),
            ))),
            lp_fee_x: Ratio::new_raw(lp_fee_x, 100000),
            lp_fee_y: Ratio::new_raw(lp_fee_y, 100000),
            treasury_fee: Ratio::new_raw(treasury_fee, 100000),
            treasury_x: TaggedAmount::new(treasury_x),
            treasury_y: TaggedAmount::new(treasury_y),
            lq_lower_bound: TaggedAmount::new(0),
            ver: FeeSwitchPoolVer::V1,
            marginal_cost: ExUnits { mem: 100, steps: 100 },
            bounds: PoolValidation {
                min_n2t_lovelace: 10000000,
                min_t2t_lovelace: 10000000,
            },
            nonce: 0,
            stake_part_script_hash: Some(ScriptHash::from([0; 28])),
        }
    }

    #[test]
    fn aggregate_swap_balancing() {
        let distinct_swaps = vec![
            500000000u64,
            10000000,
            1631500000,
            1000000,
            26520388,
            1000000000,
            420000000,
            10000000,
            1000000000,
            417494868,
            55000000,
        ];
        let aggregate_swap: u64 = distinct_swaps.iter().sum();
        let pool_0 = gen_ada_token_pool(
            1010938590871,
            3132939390433,
            0,
            99000,
            99000,
            100,
            405793826,
            1029672612,
            0,
        );
        let final_pool_distinct_swaps = distinct_swaps
            .iter()
            .fold(pool_0, |acc, x| acc.swap(Ask(*x)).fold(identity, |_| panic!()));
        let final_pool_aggregate_swap = pool_0.swap(Ask(aggregate_swap)).fold(identity, |_| panic!());
        assert_ne!(final_pool_distinct_swaps, final_pool_aggregate_swap);
        let (balanced_pool_distinct_swaps, _) =
            MakeInProgress::finalized(Trans::new(pool_0, Next::Succ(final_pool_distinct_swaps))).unwrap();
        let (balanced_pool_aggregate_swap, imbalance_aggregate_swap) =
            MakeInProgress::finalized(Trans::new(pool_0, Next::Succ(final_pool_aggregate_swap))).unwrap();
        assert_eq!(imbalance_aggregate_swap, Excess { base: 0, quote: 0 });
        assert_eq!(
            balanced_pool_distinct_swaps.0.result.fold(identity, |_| panic!()),
            final_pool_aggregate_swap
        );
        assert_eq!(
            balanced_pool_aggregate_swap.0.result.fold(identity, |_| panic!()),
            final_pool_aggregate_swap
        );
    }

    #[test]
    fn treasury_x_test() {
        let pool = gen_ada_token_pool(1632109645, 1472074052, 0, 99970, 99970, 10, 11500, 2909, 0);

        let resulted_pool = pool.swap(OnSide::Ask(900000000));
        let trans = Trans::new(pool, resulted_pool);

        assert_eq!(Some(Side::Ask), trans.trade_side());

        let correct_x_treasury = 101500;

        let Next::Succ(new_pool) = resulted_pool else {
            panic!()
        };
        assert_eq!(new_pool.treasury_x.untag(), correct_x_treasury)
    }

    struct Ctx {
        bounds: PoolValidation,
        scripts: ProtocolScriptHashes,
    }

    impl Has<DeployedScriptInfo<{ ConstFnPoolV1 as u8 }>> for Ctx {
        fn select<U: IsEqual<DeployedScriptInfo<{ ConstFnPoolV1 as u8 }>>>(
            &self,
        ) -> DeployedScriptInfo<{ ConstFnPoolV1 as u8 }> {
            self.scripts.const_fn_pool_v1
        }
    }

    impl Has<DeployedScriptInfo<{ ConstFnPoolV2 as u8 }>> for Ctx {
        fn select<U: IsEqual<DeployedScriptInfo<{ ConstFnPoolV2 as u8 }>>>(
            &self,
        ) -> DeployedScriptInfo<{ ConstFnPoolV2 as u8 }> {
            self.scripts.const_fn_pool_v2
        }
    }

    impl Has<DeployedScriptInfo<{ ConstFnPoolFeeSwitch as u8 }>> for Ctx {
        fn select<U: IsEqual<DeployedScriptInfo<{ ConstFnPoolFeeSwitch as u8 }>>>(
            &self,
        ) -> DeployedScriptInfo<{ ConstFnPoolFeeSwitch as u8 }> {
            self.scripts.const_fn_pool_fee_switch
        }
    }

    impl Has<DeployedScriptInfo<{ ConstFnPoolFeeSwitchV2 as u8 }>> for Ctx {
        fn select<U: IsEqual<DeployedScriptInfo<{ ConstFnPoolFeeSwitchV2 as u8 }>>>(
            &self,
        ) -> DeployedScriptInfo<{ ConstFnPoolFeeSwitchV2 as u8 }> {
            self.scripts.const_fn_pool_fee_switch_v2
        }
    }

    impl Has<DeployedScriptInfo<{ RoyaltyPoolV1 as u8 }>> for Ctx {
        fn select<U: IsEqual<DeployedScriptInfo<{ RoyaltyPoolV1 as u8 }>>>(
            &self,
        ) -> DeployedScriptInfo<{ RoyaltyPoolV1 as u8 }> {
            self.scripts.royalty_pool_v1
        }
    }

    impl Has<DeployedScriptInfo<{ RoyaltyPoolV1LedgerFixed as u8 }>> for Ctx {
        fn select<U: IsEqual<DeployedScriptInfo<{ RoyaltyPoolV1LedgerFixed as u8 }>>>(
            &self,
        ) -> DeployedScriptInfo<{ RoyaltyPoolV1LedgerFixed as u8 }> {
            self.scripts.royalty_pool_v1_ledger_fixed
        }
    }

    impl Has<DeployedScriptInfo<{ ConstFnPoolFeeSwitchBiDirFee as u8 }>> for Ctx {
        fn select<U: IsEqual<DeployedScriptInfo<{ ConstFnPoolFeeSwitchBiDirFee as u8 }>>>(
            &self,
        ) -> DeployedScriptInfo<{ ConstFnPoolFeeSwitchBiDirFee as u8 }> {
            self.scripts.const_fn_pool_fee_switch_bidir_fee
        }
    }

    impl Has<DeployedScriptInfo<{ RoyaltyPoolV2 as u8 }>> for Ctx {
        fn select<U: IsEqual<DeployedScriptInfo<{ RoyaltyPoolV2 as u8 }>>>(
            &self,
        ) -> DeployedScriptInfo<{ RoyaltyPoolV2 as u8 }> {
            self.scripts.royalty_pool_v2
        }
    }

    impl Has<PoolValidation> for Ctx {
        fn select<U: IsEqual<PoolValidation>>(&self) -> PoolValidation {
            self.bounds
        }
    }

    #[test]
    fn try_read_invalid_pool() {
        let raw_deployment = std::fs::read_to_string("/Users/oskin/dev/spectrum/spectrum-offchain-multiplatform/bloom-cardano-agent/resources/mainnet.deployment.json").expect("Cannot load deployment file");
        let deployment: DeployedValidators =
            serde_json::from_str(&raw_deployment).expect("Invalid deployment file");
        let scripts = ProtocolScriptHashes::from(&deployment);
        let ctx = Ctx {
            scripts,
            bounds: PoolValidation {
                min_n2t_lovelace: 150_000_000,
                min_t2t_lovelace: 10_000_000,
            },
        };
        let bearer = TransactionOutput::from_cbor_bytes(&*hex::decode(POOL_UTXO).unwrap()).unwrap();
        let pool = ConstFnPool::try_from_ledger(&bearer, &ctx);
        assert_eq!(pool, None);
    }

    const POOL_UTXO: &str = "a300583931f002facfd69d51b63e7046c6d40349b0b17c8dd775ee415c66af3cccb2f6abf60ccde92eae1a2f4fdf65f2eaf6208d872c6f0e597cc10b0701821a0115a2e9a3581cc881c20e49dbaca3ff6cef365969354150983230c39520b917f5cf7ca1444e696b65190962581c18bed14efe387074511e22c53e46433a43cbb0fdd61e3c5fbdea49f4a14b4e696b655f4144415f4c511b7fffffffffffffff581cc05d4f6397a95b48d0c8a54bf4f0d955f9638d26d7d77d02081c1591a14c4e696b655f4144415f4e465401028201d81858dcd8798bd87982581cc05d4f6397a95b48d0c8a54bf4f0d955f9638d26d7d77d02081c15914c4e696b655f4144415f4e4654d879824040d87982581cc881c20e49dbaca3ff6cef365969354150983230c39520b917f5cf7c444e696b65d87982581c18bed14efe387074511e22c53e46433a43cbb0fdd61e3c5fbdea49f44b4e696b655f4144415f4c511a00017f9818b41a0115a2e919096281d87981d87a81581cc24a311347be1bc3ebfa6f18cb14c7e6bbc2a245725fd9a8a1ccaaea00581c75c4570eb625ae881b32a34c52b159f6f3f3f2c7aaabf5bac4688133";

    #[test]
    fn available_liquidity_test_bug() {
        let fee_num = 99100;
        let reserves_x = 2562678905;
        let reserves_y = 484492;

        let pool = gen_ada_token_pool(
            reserves_x, reserves_y, 34991018, fee_num, fee_num, 90, 873404, 401, 0,
        );
        let spot = pool.static_price().unwrap().to_f64().unwrap();
        let worst_price = AbsolutePrice::new(60797, 283321878).unwrap();
        let Some(AvailableLiquidity {
            input: inp,
            output: out,
        }) = pool.available_liquidity_on_side(Bid(worst_price))
        else {
            !panic!();
        };

        let Next::Succ(pool1) = pool.swap(OnSide::Bid(60797)) else {
            panic!()
        };
        let x_rec = pool.reserves_x.untag() - pool1.reserves_x.untag();

        assert_eq!(x_rec, 283321878);
        assert_eq!(inp, 60797);
        assert_eq!(out, 283321885);
    }

    #[test]
    fn available_liquidity_low_lq_test() {
        let fee_num = 99100;
        let reserves_x = 10_000_000;
        let reserves_y = 1_000_000;

        let pool = gen_ada_token_pool(reserves_x, reserves_y, 0, fee_num, fee_num, 90, 0, 0, 0);
        let Next::Succ(pool1) = pool.swap(OnSide::Bid(100_000_000)) else {
            panic!()
        };
        let x_rec = pool.reserves_x.untag() - pool1.reserves_x.untag();
        let worst_price = AbsolutePrice::new(100_000_000, 9900010).unwrap();
        let Some(AvailableLiquidity {
            input: inp,
            output: out,
        }) = pool.available_liquidity_on_side(Bid(worst_price))
        else {
            !panic!();
        };

        assert_eq!(x_rec, 7000000);
        assert_eq!(inp, 100_000_000);
        assert_eq!(out, 7000000);

        let pool = gen_ada_token_pool(reserves_y, reserves_x, 0, fee_num, fee_num, 90, 0, 0, 1);
        let Next::Succ(pool1) = pool.swap(OnSide::Bid(100_000_000)) else {
            panic!()
        };

        let y_rec = pool.reserves_y.untag() - pool1.reserves_y.untag();
        let worst_price = AbsolutePrice::new(100_000_000, 9900010).unwrap();
        let Some(AvailableLiquidity {
            input: inp,
            output: out,
        }) = pool.available_liquidity_on_side(Bid(worst_price))
        else {
            !panic!();
        };

        assert_eq!(y_rec, 7000000);
        assert_eq!(inp, 100_000_000);
        assert_eq!(out, 7000000);
    }

    #[test]
    fn available_liquidity_test() {
        let fee_num = 98500;
        let reserves_x = 1116854094529;
        let reserves_y = 4602859113047;

        let pool = gen_ada_token_pool(reserves_x, reserves_y, 0, fee_num, fee_num, 0, 0, 0, 0);

        let worst_price = AbsolutePrice::new(4524831899687659, 1125899906842624).unwrap();
        let Some(AvailableLiquidity {
            input: _,
            output: quote_qty_ask_spot,
        }) = pool.available_liquidity_on_side(Ask(worst_price))
        else {
            !panic!()
        };

        let worst_price = AbsolutePrice::new(9007199254740992, 2113163007279601).unwrap();
        let Some(AvailableLiquidity {
            input: _,
            output: quote_qty_bid_spot,
        }) = pool.available_liquidity_on_side(Bid(worst_price))
        else {
            !panic!()
        };

        assert_eq!(quote_qty_ask_spot, 46028591130);
        assert_eq!(quote_qty_bid_spot, 20540799965)
    }
}
