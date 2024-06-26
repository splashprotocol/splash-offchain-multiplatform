use std::fmt::Debug;
use std::ops::Div;

use bignumber::BigNumber;
use cml_chain::address::Address;
use cml_chain::assets::MultiAsset;
use cml_chain::certs::StakeCredential;
use cml_chain::plutus::{ConstrPlutusData, PlutusData};
use cml_chain::transaction::{ConwayFormatTxOut, DatumOption, TransactionOutput};
use cml_chain::utils::BigInteger;
use cml_chain::Value;
use cml_multi_era::babbage::BabbageTransactionOutput;
use dashu_float::DBig;
use num_integer::Roots;
use num_rational::Ratio;
use num_traits::{CheckedAdd, CheckedSub};
use num_traits::ToPrimitive;
use type_equalities::IsEqual;

use bloom_offchain::execution_engine::liquidity_book::market_maker::{MarketMaker, PoolQuality, SpotPrice};
use bloom_offchain::execution_engine::liquidity_book::side::{Side, SideM};
use bloom_offchain::execution_engine::liquidity_book::types::AbsolutePrice;
use spectrum_cardano_lib::{TaggedAmount, TaggedAssetClass};
use spectrum_cardano_lib::ex_units::ExUnits;
use spectrum_cardano_lib::plutus_data::{
    ConstrPlutusDataExtension, DatumExtension, IntoPlutusData, PlutusDataExtension,
};
use spectrum_cardano_lib::transaction::TransactionOutputExtension;
use spectrum_cardano_lib::types::TryFromPData;
use spectrum_cardano_lib::value::ValueExtension;
use spectrum_offchain::data::{Has, Stable};
use spectrum_offchain::ledger::{IntoLedger, TryFromLedger};

use crate::constants::{FEE_DEN, LEGACY_FEE_NUM_MULTIPLIER, MAX_LQ_CAP};
use crate::data::deposit::ClassicalOnChainDeposit;
use crate::data::fee_switch_bidirectional_fee::FeeSwitchBidirectionalPoolConfig;
use crate::data::fee_switch_pool::FeeSwitchPoolConfig;
use crate::data::limit_swap::ClassicalOnChainLimitSwap;
use crate::data::operation_output::{DepositOutput, RedeemOutput, SwapOutput};
use crate::data::order::{Base, ClassicalOrder, PoolNft, Quote};
use crate::data::pair::order_canonical;
use crate::data::pool::{
    ApplyOrder, ApplyOrderError, AssetDeltas, ImmutablePoolUtxo, Lq, PoolBounds, Rx, Ry,
};
use crate::data::PoolId;
use crate::data::redeem::ClassicalOnChainRedeem;
use crate::deployment::{DeployedScriptInfo, DeployedValidator, DeployedValidatorErased, RequiresValidator};
use crate::deployment::ProtocolValidator::{
    ConstFnPoolFeeSwitch, ConstFnPoolFeeSwitchBiDirFee, ConstFnPoolV1, ConstFnPoolV2,
};
use crate::fees::FeeExtension;
use crate::pool_math::cfmm_math::{
    classic_cfmm_output_amount, classic_cfmm_reward_lp, classic_cfmm_shares_amount,
};

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
pub enum ConstFnPoolVer {
    V1,
    V2,
    FeeSwitch,
    FeeSwitchBiDirFee,
}

impl ConstFnPoolVer {
    pub fn try_from_address<Ctx>(pool_addr: &Address, ctx: &Ctx) -> Option<ConstFnPoolVer>
    where
        Ctx: Has<DeployedScriptInfo<{ ConstFnPoolV1 as u8 }>>
            + Has<DeployedScriptInfo<{ ConstFnPoolV2 as u8 }>>
            + Has<DeployedScriptInfo<{ ConstFnPoolFeeSwitch as u8 }>>
            + Has<DeployedScriptInfo<{ ConstFnPoolFeeSwitchBiDirFee as u8 }>>,
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
                return Some(ConstFnPoolVer::V1);
            } else if ctx
                .select::<DeployedScriptInfo<{ ConstFnPoolV2 as u8 }>>()
                .script_hash
                == *this_hash
            {
                return Some(ConstFnPoolVer::V2);
            } else if ctx
                .select::<DeployedScriptInfo<{ ConstFnPoolFeeSwitch as u8 }>>()
                .script_hash
                == *this_hash
            {
                return Some(ConstFnPoolVer::FeeSwitch);
            } else if ctx
                .select::<DeployedScriptInfo<{ ConstFnPoolFeeSwitchBiDirFee as u8 }>>()
                .script_hash
                == *this_hash
            {
                return Some(ConstFnPoolVer::FeeSwitchBiDirFee);
            }
        };
        None
    }
}

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub struct ConstFnPool {
    pub id: PoolId,
    pub reserves_x: TaggedAmount<Rx>,
    pub reserves_y: TaggedAmount<Ry>,
    pub liquidity: TaggedAmount<Lq>,
    pub asset_x: TaggedAssetClass<Rx>,
    pub asset_y: TaggedAssetClass<Ry>,
    pub asset_lq: TaggedAssetClass<Lq>,
    pub lp_fee_x: Ratio<u64>,
    pub lp_fee_y: Ratio<u64>,
    pub treasury_fee: Ratio<u64>,
    pub treasury_x: TaggedAmount<Rx>,
    pub treasury_y: TaggedAmount<Ry>,
    pub lq_lower_bound: TaggedAmount<Rx>,
    pub ver: ConstFnPoolVer,
    pub marginal_cost: ExUnits,
    pub min_pool_lovelace: u64,
}

impl ConstFnPool {
    pub fn get_asset_deltas(&self, side: SideM) -> AssetDeltas {
        let x = self.asset_x.untag();
        let y = self.asset_y.untag();
        let [base, _] = order_canonical(x, y);
        if base == x {
            match side {
                SideM::Bid => AssetDeltas {
                    asset_to_deduct_from: x,
                    asset_to_add_to: y,
                },
                SideM::Ask => AssetDeltas {
                    asset_to_deduct_from: y,
                    asset_to_add_to: x,
                },
            }
        } else {
            match side {
                SideM::Bid => AssetDeltas {
                    asset_to_deduct_from: y,
                    asset_to_add_to: x,
                },
                SideM::Ask => AssetDeltas {
                    asset_to_deduct_from: x,
                    asset_to_add_to: y,
                },
            }
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
        classic_cfmm_output_amount(
            self.asset_x,
            self.reserves_x - self.treasury_x,
            self.reserves_y - self.treasury_y,
            base_asset,
            base_amount,
            self.lp_fee_x - self.treasury_fee,
            self.lp_fee_y - self.treasury_fee,
        )
    }

    fn reward_lp(
        &self,
        in_x_amount: u64,
        in_y_amount: u64,
    ) -> Option<(TaggedAmount<Lq>, TaggedAmount<Rx>, TaggedAmount<Ry>)> {
        classic_cfmm_reward_lp(
            self.reserves_x - self.treasury_x,
            self.reserves_y - self.treasury_y,
            self.liquidity,
            in_x_amount,
            in_y_amount,
        )
    }

    fn shares_amount(&self, burned_lq: TaggedAmount<Lq>) -> Option<(TaggedAmount<Rx>, TaggedAmount<Ry>)> {
        classic_cfmm_shares_amount(
            self.reserves_x - self.treasury_x,
            self.reserves_y - self.treasury_y,
            self.liquidity,
            burned_lq,
        )
    }
}

impl<Ctx> RequiresValidator<Ctx> for ConstFnPool
where
    Ctx: Has<DeployedValidator<{ ConstFnPoolV1 as u8 }>>
        + Has<DeployedValidator<{ ConstFnPoolV2 as u8 }>>
        + Has<DeployedValidator<{ ConstFnPoolFeeSwitch as u8 }>>
        + Has<DeployedValidator<{ ConstFnPoolFeeSwitchBiDirFee as u8 }>>,
{
    fn get_validator(&self, ctx: &Ctx) -> DeployedValidatorErased {
        match self.ver {
            ConstFnPoolVer::V1 => ctx
                .select::<DeployedValidator<{ ConstFnPoolV1 as u8 }>>()
                .erased(),
            ConstFnPoolVer::FeeSwitch => ctx
                .select::<DeployedValidator<{ ConstFnPoolFeeSwitch as u8 }>>()
                .erased(),
            ConstFnPoolVer::FeeSwitchBiDirFee => ctx
                .select::<DeployedValidator<{ ConstFnPoolFeeSwitchBiDirFee as u8 }>>()
                .erased(),
            _ => ctx
                .select::<DeployedValidator<{ ConstFnPoolV2 as u8 }>>()
                .erased(),
        }
    }
}

impl MarketMaker for ConstFnPool {
    type U = ExUnits;

    fn static_price(&self) -> SpotPrice {
        let x = self.asset_x.untag();
        let y = self.asset_y.untag();
        let [base, _] = order_canonical(x, y);
        let total_fee_num = self.lp_fee_y - self.treasury_fee;
        if x == base {
            AbsolutePrice::new(
                self.reserves_y.untag() * total_fee_num.numer(),
                self.reserves_x.untag() * total_fee_num.denom(),
            )
            .into()
        } else {
            AbsolutePrice::new(self.reserves_x.untag(), self.reserves_y.untag()).into()
        }
    }

    fn real_price(&self, input: Side<u64>) -> AbsolutePrice {
        let x = self.asset_x.untag();
        let y = self.asset_y.untag();
        let [base, quote] = order_canonical(x, y);
        let (base, quote) = match input {
            Side::Bid(input) => (
                self.output_amount(TaggedAssetClass::new(quote), TaggedAmount::new(input))
                    .untag(),
                input,
            ),
            Side::Ask(input) => (
                input,
                self.output_amount(TaggedAssetClass::new(base), TaggedAmount::new(input))
                    .untag(),
            ),
        };
        AbsolutePrice::new(quote, base)
    }

    fn swap(mut self, input: Side<u64>) -> (u64, Self) {
        let x = self.asset_x.untag();
        let y = self.asset_y.untag();
        let [base, quote] = order_canonical(x, y);
        let output = match input {
            Side::Bid(input) => self
                .output_amount(TaggedAssetClass::new(quote), TaggedAmount::new(input))
                .untag(),
            Side::Ask(input) => self
                .output_amount(TaggedAssetClass::new(base), TaggedAmount::new(input))
                .untag(),
        };
        let (base_reserves, base_treasury, quote_reserves, quote_treasury) = if x == base {
            (
                self.reserves_x.as_mut(),
                self.treasury_x.as_mut(),
                self.reserves_y.as_mut(),
                self.treasury_y.as_mut(),
            )
        } else {
            (
                self.reserves_y.as_mut(),
                self.treasury_y.as_mut(),
                self.reserves_x.as_mut(),
                self.treasury_x.as_mut(),
            )
        };
        match input {
            Side::Bid(input) => {
                // A user bid means that they wish to buy the base asset for the quote asset, hence
                // pool reserves of base decreases while reserves of quote increase.
                *quote_reserves += input;
                *base_reserves -= output;
                *quote_treasury += (input * self.treasury_fee.numer()) / self.treasury_fee.denom();
                (output, self)
            }
            Side::Ask(input) => {
                // User ask is the opposite; sell the base asset for the quote asset.
                *base_reserves += input;
                *quote_reserves -= output;
                *base_treasury += (input * self.treasury_fee.numer()) / self.treasury_fee.denom();
                (output, self)
            }
        }
    }

    fn quality(&self) -> PoolQuality {
        let lq = self.reserves_x.untag() as u128 * self.reserves_y.untag() as u128;
        PoolQuality::from(lq.sqrt())
    }

    fn marginal_cost_hint(&self) -> Self::U {
        self.marginal_cost
    }

    fn is_active(&self) -> bool {
        let lq_bound = (self.reserves_x.untag() * 2) >= self.lq_lower_bound.untag();
        let bot_bound = if self.asset_x.is_native() {
            self.reserves_x.untag() >= self.min_pool_lovelace
        } else {
            self.reserves_y.untag() >= self.min_pool_lovelace
        };
        lq_bound && bot_bound
    }

    fn available_liquidity_by_user_impact(&self, max_user_price_impact: Side<Ratio<u64>>) -> (u64, u64) {
        // "max_user_price_impact" is calculated as
        // "max_user_price_impact = 1 - avg_sell_price / market_price" to be always > 0.
        // Outputs are ("quote_amount_available", "base_amount_required").
        // Outputs reflects how many quote asset the user will receive and how many base asset
        // must be added to the pool in order for this operation to occur with a given
        // "max_price_impact" relative to the current state of the pool.
        // Note: all calculations are made taking fees into account, thus "max_user_price_impact"
        // must also include fees in "market_price" calculation.
        const BN_ONE: BigNumber = BigNumber { value: DBig::ONE };
        let sqrt_degree = BigNumber::from(0.5);

        let (tradable_reserves_base, tradable_reserves_quote, total_fee_mult) = match max_user_price_impact {
            Side::Bid(_) => (
                BigNumber::from((self.reserves_y - self.treasury_y).untag() as f64),
                BigNumber::from((self.reserves_x - self.treasury_x).untag() as f64),
                BigNumber::from((self.lp_fee_y - self.treasury_fee).to_f64().unwrap()),
            ),
            Side::Ask(_) => (
                BigNumber::from((self.reserves_x - self.treasury_x).untag() as f64),
                BigNumber::from((self.reserves_y - self.treasury_y).untag() as f64),
                BigNumber::from((self.lp_fee_x - self.treasury_fee).to_f64().unwrap()),
            ),
        };

        let lq_balance = (tradable_reserves_base.clone() * tradable_reserves_quote.clone()).pow(&sqrt_degree);

        let market_price = tradable_reserves_quote
            .clone()
            .div(tradable_reserves_base.clone())
            * total_fee_mult.clone();

        let impact_price = market_price
            * (BN_ONE
                - BigNumber::from(*max_user_price_impact.unwrap().numer() as f64)
                    .div(BigNumber::from(*max_user_price_impact.unwrap().denom() as f64)))
            .div(total_fee_mult.clone());

        let p1 =
            (impact_price * lq_balance.clone() / tradable_reserves_quote.clone()).pow(&(BigNumber::from(2)));
        let p1_sqrt = p1.clone().pow(&sqrt_degree);
        let x1 = lq_balance.clone() / p1_sqrt.clone();
        let y1 = lq_balance.clone() * p1_sqrt.clone();

        let base = (x1.clone() - tradable_reserves_base.clone()) / total_fee_mult;
        let quote = tradable_reserves_quote - y1.clone();

        return (
            <u64>::try_from(quote.to_precision(0).value.to_int().value()).unwrap(),
            <u64>::try_from(base.to_precision(0).value.to_int().value()).unwrap(),
        );
    }
    fn available_liquidity_by_spot_impact(&self, max_spot_price_impact: Side<Ratio<u64>>) -> (u64, u64) {
        // "max_spot_price_impact" is calculated as
        // "max_spot_price_impact = 1 - market_price_new / market_price" to be always > 0.
        // Outputs are ("quote_amount_available", "base_amount_required").
        // Outputs reflects how many quote asset the user will receive and how many base asset
        // must be added to the pool in order for this operation to occur with a given
        // "max_spot_price_impact" relative to the current state of the pool.
        // Note: all calculations are made taking fees into account, thus "max_spot_price_impact"
        // must also include fees in "market_price" calculation.
        const BN_ONE: BigNumber = BigNumber { value: DBig::ONE };
        let sqrt_degree = BigNumber::from(0.5);

        let (tradable_reserves_base, tradable_reserves_quote, total_fee_mult) = match max_spot_price_impact {
            Side::Bid(_) => (
                BigNumber::from((self.reserves_y - self.treasury_y).untag() as f64),
                BigNumber::from((self.reserves_x - self.treasury_x).untag() as f64),
                BigNumber::from((self.lp_fee_y - self.treasury_fee).to_f64().unwrap()),
            ),
            Side::Ask(_) => (
                BigNumber::from((self.reserves_x - self.treasury_x).untag() as f64),
                BigNumber::from((self.reserves_y - self.treasury_y).untag() as f64),
                BigNumber::from((self.lp_fee_x - self.treasury_fee).to_f64().unwrap()),
            ),
        };

        let lq_balance = (tradable_reserves_base.clone() * tradable_reserves_quote.clone()).pow(&sqrt_degree);

        let market_price = tradable_reserves_quote
            .clone()
            .div(tradable_reserves_base.clone())
            * total_fee_mult.clone();

        let impact_price = market_price
            * (BN_ONE
                - BigNumber::from(*max_spot_price_impact.unwrap().numer() as f64)
                    .div(BigNumber::from(*max_spot_price_impact.unwrap().denom() as f64)))
            .div(total_fee_mult.clone());

        let p1_sqrt = impact_price.clone().pow(&sqrt_degree);
        let x1 = lq_balance.clone() / p1_sqrt.clone();
        let y1 = lq_balance.clone() * p1_sqrt.clone();

        let base = (x1.clone() - tradable_reserves_base.clone()) / total_fee_mult;
        let quote = tradable_reserves_quote - y1.clone();

        return (
            <u64>::try_from(quote.to_precision(0).value.to_int().value()).unwrap(),
            <u64>::try_from(base.to_precision(0).value.to_int().value()).unwrap(),
        );
    }
}

impl Has<ConstFnPoolVer> for ConstFnPool {
    fn select<U: IsEqual<ConstFnPoolVer>>(&self) -> ConstFnPoolVer {
        self.ver
    }
}

impl Stable for ConstFnPool {
    type StableId = PoolId;
    fn stable_id(&self) -> Self::StableId {
        self.id
    }
    fn is_quasi_permanent(&self) -> bool {
        true
    }
}

impl<Ctx> TryFromLedger<BabbageTransactionOutput, Ctx> for ConstFnPool
where
    Ctx: Has<DeployedScriptInfo<{ ConstFnPoolV1 as u8 }>>
        + Has<DeployedScriptInfo<{ ConstFnPoolV2 as u8 }>>
        + Has<DeployedScriptInfo<{ ConstFnPoolFeeSwitch as u8 }>>
        + Has<DeployedScriptInfo<{ ConstFnPoolFeeSwitchBiDirFee as u8 }>>
        + Has<PoolBounds>,
{
    fn try_from_ledger(repr: &BabbageTransactionOutput, ctx: &Ctx) -> Option<Self> {
        if let Some(pool_ver) = ConstFnPoolVer::try_from_address(repr.address(), ctx) {
            let value = repr.value();
            let pd = repr.datum().clone()?.into_pd()?;
            let bounds = ctx.select::<PoolBounds>();
            let marginal_cost = match pool_ver {
                ConstFnPoolVer::V1 => {
                    ctx.select::<DeployedScriptInfo<{ ConstFnPoolV1 as u8 }>>()
                        .marginal_cost
                }
                ConstFnPoolVer::V2 => {
                    ctx.select::<DeployedScriptInfo<{ ConstFnPoolV2 as u8 }>>()
                        .marginal_cost
                }
                ConstFnPoolVer::FeeSwitch => {
                    ctx.select::<DeployedScriptInfo<{ ConstFnPoolFeeSwitch as u8 }>>()
                        .marginal_cost
                }
                ConstFnPoolVer::FeeSwitchBiDirFee => {
                    ctx.select::<DeployedScriptInfo<{ ConstFnPoolFeeSwitchBiDirFee as u8 }>>()
                        .marginal_cost
                }
            };
            return match pool_ver {
                ConstFnPoolVer::V1 | ConstFnPoolVer::V2 => {
                    let conf = LegacyCFMMPoolConfig::try_from_pd(pd.clone())?;
                    let liquidity_neg = value.amount_of(conf.asset_lq.into())?;
                    Some(ConstFnPool {
                        id: PoolId::try_from(conf.pool_nft).ok()?,
                        reserves_x: TaggedAmount::new(value.amount_of(conf.asset_x.into())?),
                        reserves_y: TaggedAmount::new(value.amount_of(conf.asset_y.into())?),
                        liquidity: TaggedAmount::new(MAX_LQ_CAP - liquidity_neg),
                        asset_x: conf.asset_x,
                        asset_y: conf.asset_y,
                        asset_lq: conf.asset_lq,
                        // legacy lp fee den = 1000
                        // new lp fee den = 100000
                        lp_fee_x: Ratio::new_raw(conf.lp_fee_num * LEGACY_FEE_NUM_MULTIPLIER, FEE_DEN),
                        lp_fee_y: Ratio::new_raw(conf.lp_fee_num * LEGACY_FEE_NUM_MULTIPLIER, FEE_DEN),
                        treasury_fee: Ratio::new_raw(0, 1),
                        treasury_x: TaggedAmount::new(0),
                        treasury_y: TaggedAmount::new(0),
                        lq_lower_bound: conf.lq_lower_bound,
                        ver: pool_ver,
                        marginal_cost,
                        min_pool_lovelace: bounds.min_lovelace,
                    })
                }
                ConstFnPoolVer::FeeSwitch => {
                    let conf = FeeSwitchPoolConfig::try_from_pd(pd.clone())?;
                    let liquidity_neg = value.amount_of(conf.asset_lq.into())?;
                    Some(ConstFnPool {
                        id: PoolId::try_from(conf.pool_nft).ok()?,
                        reserves_x: TaggedAmount::new(value.amount_of(conf.asset_x.into())?),
                        reserves_y: TaggedAmount::new(value.amount_of(conf.asset_y.into())?),
                        liquidity: TaggedAmount::new(MAX_LQ_CAP - liquidity_neg),
                        asset_x: conf.asset_x,
                        asset_y: conf.asset_y,
                        asset_lq: conf.asset_lq,
                        lp_fee_x: Ratio::new_raw(conf.lp_fee_num, FEE_DEN),
                        lp_fee_y: Ratio::new_raw(conf.lp_fee_num, FEE_DEN),
                        treasury_fee: Ratio::new_raw(conf.treasury_fee_num, FEE_DEN),
                        treasury_x: TaggedAmount::new(conf.treasury_x),
                        treasury_y: TaggedAmount::new(conf.treasury_y),
                        lq_lower_bound: conf.lq_lower_bound,
                        ver: pool_ver,
                        marginal_cost,
                        min_pool_lovelace: bounds.min_lovelace,
                    })
                }
                ConstFnPoolVer::FeeSwitchBiDirFee => {
                    let conf = FeeSwitchBidirectionalPoolConfig::try_from_pd(pd.clone())?;
                    let liquidity_neg = value.amount_of(conf.asset_lq.into())?;
                    Some(ConstFnPool {
                        id: PoolId::try_from(conf.pool_nft).ok()?,
                        reserves_x: TaggedAmount::new(value.amount_of(conf.asset_x.into())?)
                            - TaggedAmount::new(conf.treasury_x),
                        reserves_y: TaggedAmount::new(value.amount_of(conf.asset_y.into())?)
                            - TaggedAmount::new(conf.treasury_y),
                        liquidity: TaggedAmount::new(MAX_LQ_CAP - liquidity_neg),
                        asset_x: conf.asset_x,
                        asset_y: conf.asset_y,
                        asset_lq: conf.asset_lq,
                        lp_fee_x: Ratio::new_raw(conf.lp_fee_num_x, FEE_DEN),
                        lp_fee_y: Ratio::new_raw(conf.lp_fee_num_y, FEE_DEN),
                        treasury_fee: Ratio::new_raw(conf.treasury_fee_num, FEE_DEN),
                        treasury_x: TaggedAmount::new(conf.treasury_x),
                        treasury_y: TaggedAmount::new(conf.treasury_y),
                        lq_lower_bound: conf.lq_lower_bound,
                        ver: pool_ver,
                        marginal_cost,
                        min_pool_lovelace: bounds.min_lovelace,
                    })
                }
            };
        };
        None
    }
}

impl IntoLedger<TransactionOutput, ImmutablePoolUtxo> for ConstFnPool {
    fn into_ledger(self, mut immut_pool: ImmutablePoolUtxo) -> TransactionOutput {
        let mut ma = MultiAsset::new();
        let coins = if self.asset_x.is_native() {
            let (policy, name) = self.asset_y.untag().into_token().unwrap();
            ma.set(policy, name.into(), self.reserves_y.untag());
            self.reserves_x.untag()
        } else if self.asset_y.is_native() {
            let (policy, name) = self.asset_x.untag().into_token().unwrap();
            ma.set(policy, name.into(), self.reserves_x.untag());
            self.reserves_y.untag()
        } else {
            let (policy_x, name_x) = self.asset_x.untag().into_token().unwrap();
            ma.set(policy_x, name_x.into(), self.reserves_x.untag());
            let (policy_y, name_y) = self.asset_y.untag().into_token().unwrap();
            ma.set(policy_y, name_y.into(), self.reserves_y.untag());
            immut_pool.value
        };
        let (policy_lq, name_lq) = self.asset_lq.untag().into_token().unwrap();
        let (nft_lq, name_nft) = self.id.into();
        ma.set(policy_lq, name_lq.into(), MAX_LQ_CAP - self.liquidity.untag());
        ma.set(nft_lq, name_nft.into(), 1);

        if self.ver == ConstFnPoolVer::FeeSwitch {
            if let Some(DatumOption::Datum { datum, .. }) = &mut immut_pool.datum_option {
                unsafe_update_pd(datum, self.treasury_x.untag(), self.treasury_y.untag());
            }
        }

        TransactionOutput::new_conway_format_tx_out(ConwayFormatTxOut {
            address: immut_pool.address,
            amount: Value::new(coins, ma),
            datum_option: immut_pool.datum_option,
            script_reference: immut_pool.script_reference,
            encodings: None,
        })
    }
}

pub fn unsafe_update_pd(data: &mut PlutusData, treasury_x: u64, treasury_y: u64) {
    let cpd = data.get_constr_pd_mut().unwrap();
    cpd.set_field(6, treasury_x.into_pd());
    cpd.set_field(7, treasury_y.into_pd());
}

impl ApplyOrder<ClassicalOnChainLimitSwap> for ConstFnPool {
    type Result = SwapOutput;

    fn apply_order(
        mut self,
        ClassicalOrder { id, pool_id, order }: ClassicalOnChainLimitSwap,
    ) -> Result<(Self, SwapOutput), ApplyOrderError<ClassicalOnChainLimitSwap>> {
        let quote_amount = self.output_amount(order.base_asset, order.base_amount);
        if quote_amount < order.min_expected_quote_amount {
            return Err(ApplyOrderError::slippage(
                ClassicalOrder {
                    id,
                    pool_id,
                    order: order.clone(),
                },
                quote_amount,
                order.clone().min_expected_quote_amount,
            ));
        }
        // Adjust pool value.
        if order.quote_asset.untag() == self.asset_x.untag() {
            let additional_treasury_y = (((order.base_amount.untag() as u128)
                * (*self.treasury_fee.numer() as u128))
                / (*self.treasury_fee.denom() as u128)) as u64;
            self.reserves_x = self.reserves_x - quote_amount.retag();
            self.treasury_y = self.treasury_y + TaggedAmount::new(additional_treasury_y);
            self.reserves_y = self.reserves_y + order.base_amount.retag();
        } else {
            let additional_treasury_x = (((order.base_amount.untag() as u128)
                * (*self.treasury_fee.numer() as u128))
                / (*self.treasury_fee.denom() as u128)) as u64;
            self.treasury_x = self.treasury_x + TaggedAmount::new(additional_treasury_x);
            self.reserves_y = self.reserves_y - quote_amount.retag();
            self.reserves_x = self.reserves_x + order.base_amount.retag();
        }
        // Prepare user output.
        let batcher_fee = order.fee.value().linear_fee(quote_amount.untag());
        if batcher_fee > order.ada_deposit {
            return Err(ApplyOrderError::low_batcher_fee(
                ClassicalOrder {
                    id,
                    pool_id,
                    order: order.clone(),
                },
                batcher_fee,
                order.clone().ada_deposit,
            ));
        }
        let ada_residue = order.ada_deposit - batcher_fee;
        let swap_output = SwapOutput {
            quote_asset: order.quote_asset,
            quote_amount,
            ada_residue,
            redeemer_pkh: order.redeemer_pkh,
            redeemer_stake_pkh: order.redeemer_stake_pkh,
        };
        // Prepare batcher fee.
        Ok((self, swap_output))
    }
}

impl ApplyOrder<ClassicalOnChainDeposit> for ConstFnPool {
    type Result = DepositOutput;

    fn apply_order(
        mut self,
        deposit: ClassicalOnChainDeposit,
    ) -> Result<(Self, DepositOutput), ApplyOrderError<ClassicalOnChainDeposit>> {
        let order = deposit.order;
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

        match self.reward_lp(net_x, net_y) {
            Some((unlocked_lq, change_x, change_y)) => {
                self.reserves_x = self
                    .reserves_x
                    .checked_add(&TaggedAmount::new(net_x))
                    .and_then(|result| result.checked_sub(&change_x))
                    .ok_or(ApplyOrderError::incompatible(deposit.clone()))?;
                self.reserves_y = self
                    .reserves_y
                    .checked_add(&TaggedAmount::new(net_y))
                    .and_then(|result| result.checked_sub(&change_y))
                    .ok_or(ApplyOrderError::incompatible(deposit.clone()))?;
                self.liquidity = self
                    .liquidity
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

                Ok((self, deposit_output))
            }
            None => Err(ApplyOrderError::incompatible(deposit)),
        }
    }
}

impl ApplyOrder<ClassicalOnChainRedeem> for ConstFnPool {
    type Result = RedeemOutput;

    fn apply_order(
        mut self,
        redeem: ClassicalOnChainRedeem,
    ) -> Result<(Self, RedeemOutput), ApplyOrderError<ClassicalOnChainRedeem>> {
        let order = redeem.order;
        match self.shares_amount(order.token_lq_amount) {
            Some((x_amount, y_amount)) => {
                self.reserves_x = self
                    .reserves_x
                    .checked_sub(&x_amount)
                    .ok_or(ApplyOrderError::incompatible(redeem.clone()))?;
                self.reserves_y = self
                    .reserves_y
                    .checked_sub(&y_amount)
                    .ok_or(ApplyOrderError::incompatible(redeem.clone()))?;
                self.liquidity = self
                    .liquidity
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

                Ok((self, redeem_output))
            }
            None => Err(ApplyOrderError::incompatible(redeem)),
        }
    }
}

mod tests {
    use cml_crypto::ScriptHash;
    use num_rational::Ratio;
    use num_traits::ToPrimitive;

    use bloom_offchain::execution_engine::liquidity_book::market_maker::MarketMaker;
    use bloom_offchain::execution_engine::liquidity_book::side::Side;
    use bloom_offchain::execution_engine::liquidity_book::side::Side::{Ask, Bid};
    use spectrum_cardano_lib::{AssetClass, AssetName, TaggedAmount, TaggedAssetClass};
    use spectrum_cardano_lib::ex_units::ExUnits;

    use crate::data::cfmm_pool::{ConstFnPool, ConstFnPoolVer};
    use crate::data::PoolId;

    fn gen_ada_token_pool(
        reserves_x: u64,
        reserves_y: u64,
        liquidity: u64,
        lp_fee_x: u64,
        lp_fee_y: u64,
        treasury_fee: u64,
        treasury_x: u64,
        treasury_y: u64,
    ) -> ConstFnPool {
        return ConstFnPool {
            id: PoolId::from((
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
            asset_x: TaggedAssetClass::new(AssetClass::Native),
            asset_y: TaggedAssetClass::new(AssetClass::Token((
                ScriptHash::from([
                    75, 52, 89, 253, 24, 161, 219, 171, 226, 7, 205, 25, 201, 149, 26, 159, 172, 159, 92, 15,
                    156, 56, 78, 61, 151, 239, 186, 38,
                ]),
                AssetName::from((
                    5,
                    [
                        116, 101, 115, 116, 67, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
                        0, 0, 0, 0, 0, 0, 0,
                    ],
                )),
            ))),
            asset_lq: TaggedAssetClass::new(AssetClass::Token((
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
            ver: ConstFnPoolVer::FeeSwitch,
            marginal_cost: ExUnits { mem: 100, steps: 100 },
            min_pool_lovelace: 10000000,
        };
    }

    #[test]
    fn treasury_x_test() {
        let pool = gen_ada_token_pool(1632109645, 1472074052, 0, 99970, 99970, 10, 11500, 2909);

        let (_, new_pool) = pool.clone().swap(Side::Ask(900000000));

        let correct_x_treasury = 101500;

        assert_eq!(new_pool.treasury_x.untag(), correct_x_treasury)
    }

    #[test]
    fn available_liquidity_test() {
        let fee_num = 98500;
        let reserves_x = 1116854094529;
        let reserves_y = 4602859113047;

        let pool = gen_ada_token_pool(reserves_x, reserves_y, 0, fee_num, fee_num, 0, 0, 0);

        // Repair available volumes from average sell price impact.
        let max_target_price_impact = Ratio::new_raw(45035996273705, 4503599627370496);

        let (quote_qty_ask, _) = pool.available_liquidity_by_user_impact(Ask(max_target_price_impact));

        let (quote_qty_bid, _) = pool.available_liquidity_by_user_impact(Bid(max_target_price_impact));

        assert_eq!(quote_qty_ask, 46028591130);
        assert_eq!(quote_qty_bid, 11168540945);

        // Repair available volumes from pool spot price impact.
        let max_spot_price_impact = Ratio::new_raw(179243265169347, 9007199254740992);

        let (quote_qty_ask_spot, _) = pool.available_liquidity_by_spot_impact(Ask(max_spot_price_impact));

        let (quote_qty_bid_spot, _) = pool.available_liquidity_by_spot_impact(Bid(max_spot_price_impact));

        assert_eq!(quote_qty_ask_spot, 46028591130);
        assert_eq!(quote_qty_bid_spot, 11168540945)
    }
    #[test]
    fn best_price_selection_test() {
        let fee_num0 = 98500;
        let reserves_x0 = 1416854094529;
        let reserves_y0 = 4202859113047;

        let fee_num1 = 96500;
        let reserves_x1 = 1716854094529;
        let reserves_y1 = 4602859113047;

        let fee_num2 = 99700;
        let reserves_x2 = 1816854094523;
        let reserves_y2 = 5102859113047;

        let pool0 = gen_ada_token_pool(reserves_x0, reserves_y0, 0, fee_num0, fee_num0, 0, 0, 0);
        let pool1 = gen_ada_token_pool(reserves_x1, reserves_y1, 0, fee_num1, fee_num1, 0, 0, 0);
        let pool2 = gen_ada_token_pool(reserves_x2, reserves_y2, 0, fee_num2, fee_num2, 0, 0, 0);
        let candidates = [pool0.clone(), pool1.clone(), pool2.clone()];
        // Step 1. Compare spot price
        let mut spot_prices = vec![];
        for p in candidates {
            spot_prices.push(p.static_price().unwrap().to_f64().unwrap())
        }
        let a = 3;
        assert_eq!(spot_prices, vec![1f64])
    }
}
