use crate::constants::{FEE_DEN, MAX_LQ_CAP};
use crate::data::cfmm_pool::AMMOps;
use crate::data::order::{Base, PoolNft, Quote};
use crate::data::pair::order_canonical;
use crate::data::pool::{ImmutablePoolUtxo, Lq, PoolValidation, Rx, Ry};
use crate::data::PoolId;
use crate::deployment::ProtocolValidator::{
    ConstFnPoolFeeSwitch, ConstFnPoolFeeSwitchBiDirFee, ConstFnPoolFeeSwitchV2,
};
use crate::deployment::{DeployedScriptInfo, DeployedValidator, DeployedValidatorErased, RequiresValidator};
use crate::pool_math::cfmm_math::{
    classic_cfmm_output_amount, classic_cfmm_reward_lp, classic_cfmm_shares_amount,
    UNTOUCHABLE_LOVELACE_AMOUNT,
};
use bignumber::BigNumber;
use bloom_offchain::execution_engine::liquidity_book::core::Next;
use bloom_offchain::execution_engine::liquidity_book::market_maker::{
    AbsoluteReserves, AvailableLiquidity, MakerBehavior, MarketMaker, PoolQuality, SpotPrice,
};
use bloom_offchain::execution_engine::liquidity_book::side::OnSide;
use bloom_offchain::execution_engine::liquidity_book::types::AbsolutePrice;
use cml_chain::address::Address;
use cml_chain::assets::MultiAsset;
use cml_chain::certs::{Credential, StakeCredential};
use cml_chain::plutus::PlutusData;
use cml_chain::transaction::{ConwayFormatTxOut, DatumOption, TransactionOutput};
use cml_chain::Value;
use cml_crypto::ScriptHash;
use num_rational::Ratio;
use num_traits::{CheckedSub, ToPrimitive};
use spectrum_cardano_lib::address::AddressExtension;
use spectrum_cardano_lib::ex_units::ExUnits;
use spectrum_cardano_lib::plutus_data::{ConstrPlutusDataExtension, DatumExtension};
use spectrum_cardano_lib::plutus_data::{IntoPlutusData, PlutusDataExtension};
use spectrum_cardano_lib::transaction::TransactionOutputExtension;
use spectrum_cardano_lib::types::TryFromPData;
use spectrum_cardano_lib::value::ValueExtension;
use spectrum_cardano_lib::AssetClass::Native;
use spectrum_cardano_lib::{TaggedAmount, TaggedAssetClass, Token};
use spectrum_offchain::domain::{Has, Stable};
use spectrum_offchain::ledger::{IntoLedger, TryFromLedger};
use std::fmt::{Display, Formatter};
use std::ops::Div;
use void::Void;

pub struct FeeSwitchPoolConfig {
    pub pool_nft: TaggedAssetClass<PoolNft>,
    pub asset_x: TaggedAssetClass<Rx>,
    pub asset_y: TaggedAssetClass<Ry>,
    pub asset_lq: TaggedAssetClass<Lq>,
    pub lp_fee_num: u64,
    pub treasury_fee_num: u64,
    pub treasury_x: u64,
    pub treasury_y: u64,
    pub lq_lower_bound: TaggedAmount<Rx>,
}

impl TryFromPData for FeeSwitchPoolConfig {
    fn try_from_pd(data: PlutusData) -> Option<Self> {
        let mut cpd = data.into_constr_pd()?;
        Some(Self {
            pool_nft: TaggedAssetClass::try_from_pd(cpd.take_field(0)?)?,
            asset_x: TaggedAssetClass::try_from_pd(cpd.take_field(1)?)?,
            asset_y: TaggedAssetClass::try_from_pd(cpd.take_field(2)?)?,
            asset_lq: TaggedAssetClass::try_from_pd(cpd.take_field(3)?)?,
            lp_fee_num: cpd.take_field(4)?.into_u64()?,
            treasury_fee_num: cpd.take_field(5)?.into_u64()?,
            treasury_x: cpd.take_field(6)?.into_u64()?,
            treasury_y: cpd.take_field(7)?.into_u64()?,
            lq_lower_bound: TaggedAmount::new(cpd.take_field(9).and_then(|pd| pd.into_u64()).unwrap_or(0)),
        })
    }
}

pub struct FeeSwitchBidirectionalPoolConfig {
    pub pool_nft: TaggedAssetClass<PoolNft>,
    pub asset_x: TaggedAssetClass<Rx>,
    pub asset_y: TaggedAssetClass<Ry>,
    pub asset_lq: TaggedAssetClass<Lq>,
    pub lp_fee_num_x: u64,
    pub lp_fee_num_y: u64,
    pub treasury_fee_num: u64,
    pub treasury_x: u64,
    pub treasury_y: u64,
    pub lq_lower_bound: TaggedAmount<Rx>,
}

impl TryFromPData for FeeSwitchBidirectionalPoolConfig {
    fn try_from_pd(data: PlutusData) -> Option<Self> {
        let mut cpd = data.into_constr_pd()?;
        Some(Self {
            pool_nft: TaggedAssetClass::try_from_pd(cpd.take_field(0)?)?,
            asset_x: TaggedAssetClass::try_from_pd(cpd.take_field(1)?)?,
            asset_y: TaggedAssetClass::try_from_pd(cpd.take_field(2)?)?,
            asset_lq: TaggedAssetClass::try_from_pd(cpd.take_field(3)?)?,
            lp_fee_num_x: cpd.take_field(4)?.into_u64()?,
            lp_fee_num_y: cpd.take_field(5)?.into_u64()?,
            treasury_fee_num: cpd.take_field(6)?.into_u64()?,
            treasury_x: cpd.take_field(7)?.into_u64()?,
            treasury_y: cpd.take_field(8)?.into_u64()?,
            lq_lower_bound: TaggedAmount::new(cpd.take_field(10).and_then(|pd| pd.into_u64()).unwrap_or(0)),
        })
    }
}

pub fn unsafe_update_pd_fee_switch(data: &mut PlutusData, treasury_x: u64, treasury_y: u64) {
    let cpd = data.get_constr_pd_mut().unwrap();
    cpd.set_field(6, treasury_x.into_pd());
    cpd.set_field(7, treasury_y.into_pd());
}

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub enum FeeSwitchPoolVer {
    V1,
    V2,
    BiDirV1,
}

impl FeeSwitchPoolVer {
    pub fn try_from_address<Ctx>(pool_addr: &Address, ctx: &Ctx) -> Option<FeeSwitchPoolVer>
    where
        Ctx: Has<DeployedScriptInfo<{ ConstFnPoolFeeSwitch as u8 }>>
            + Has<DeployedScriptInfo<{ ConstFnPoolFeeSwitchV2 as u8 }>>
            + Has<DeployedScriptInfo<{ ConstFnPoolFeeSwitchBiDirFee as u8 }>>,
    {
        let maybe_hash = pool_addr.payment_cred().and_then(|c| match c {
            StakeCredential::PubKey { .. } => None,
            StakeCredential::Script { hash, .. } => Some(hash),
        });
        if let Some(this_hash) = maybe_hash {
            if ctx
                .select::<DeployedScriptInfo<{ ConstFnPoolFeeSwitch as u8 }>>()
                .script_hash
                == *this_hash
            {
                return Some(FeeSwitchPoolVer::V1);
            } else if ctx
                .select::<DeployedScriptInfo<{ ConstFnPoolFeeSwitchV2 as u8 }>>()
                .script_hash
                == *this_hash
            {
                return Some(FeeSwitchPoolVer::V2);
            } else if ctx
                .select::<DeployedScriptInfo<{ ConstFnPoolFeeSwitchBiDirFee as u8 }>>()
                .script_hash
                == *this_hash
            {
                return Some(FeeSwitchPoolVer::BiDirV1);
            }
        };
        None
    }
}

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub struct FeeSwitchPool {
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
    pub ver: FeeSwitchPoolVer,
    pub marginal_cost: ExUnits,
    pub bounds: PoolValidation,
    pub nonce: u64,
    pub stake_part_script_hash: Option<ScriptHash>,
}

impl Display for FeeSwitchPool {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(&*format!(
            "FeeSwitchPool(id: {}, static_price: {}, rx: {}, ry: {},  tx: {}, ty: {})",
            self.id,
            self.static_price(),
            self.reserves_x,
            self.reserves_y,
            self.treasury_x,
            self.treasury_y
        ))
    }
}

impl<Ctx> TryFromLedger<TransactionOutput, Ctx> for FeeSwitchPool
where
    Ctx: Has<DeployedScriptInfo<{ ConstFnPoolFeeSwitch as u8 }>>
        + Has<DeployedScriptInfo<{ ConstFnPoolFeeSwitchV2 as u8 }>>
        + Has<DeployedScriptInfo<{ ConstFnPoolFeeSwitchBiDirFee as u8 }>>
        + Has<PoolValidation>,
{
    fn try_from_ledger(repr: &TransactionOutput, ctx: &Ctx) -> Option<Self> {
        if let Some(pool_ver) = FeeSwitchPoolVer::try_from_address(repr.address(), ctx) {
            let value = repr.value();
            let pd = repr.datum().clone()?.into_pd()?;
            let bounds = ctx.select::<PoolValidation>();
            let stake_part = repr
                .address()
                .staking_cred()
                .and_then(|staking_cred| match staking_cred {
                    StakeCredential::Script { hash, .. } => Some(*hash),
                    _ => None,
                });
            let marginal_cost = match pool_ver {
                FeeSwitchPoolVer::V1 => {
                    ctx.select::<DeployedScriptInfo<{ ConstFnPoolFeeSwitch as u8 }>>()
                        .marginal_cost
                }
                FeeSwitchPoolVer::V2 => {
                    ctx.select::<DeployedScriptInfo<{ ConstFnPoolFeeSwitchV2 as u8 }>>()
                        .marginal_cost
                }
                FeeSwitchPoolVer::BiDirV1 => {
                    ctx.select::<DeployedScriptInfo<{ ConstFnPoolFeeSwitchBiDirFee as u8 }>>()
                        .marginal_cost
                }
            };
            match pool_ver {
                FeeSwitchPoolVer::V1 | FeeSwitchPoolVer::V2 => {
                    let conf = FeeSwitchPoolConfig::try_from_pd(pd.clone())?;
                    let liquidity_neg = value.amount_of(conf.asset_lq.into())?;
                    let lov = value.amount_of(Native)?;
                    let reserves_x = value.amount_of(conf.asset_x.into())?;
                    let reserves_y = value.amount_of(conf.asset_y.into())?;
                    let pure_reserves_x = reserves_x - conf.treasury_x;
                    let pure_reserves_y = reserves_y - conf.treasury_y;
                    let non_empty_reserves = pure_reserves_x > 0 && pure_reserves_y > 0;
                    let sufficient_lovelace = conf.asset_x.is_native()
                        || conf.asset_y.is_native()
                        || bounds.min_t2t_lovelace <= lov;
                    let enough_reserves = reserves_x > conf.treasury_x && reserves_y > conf.treasury_y;
                    if non_empty_reserves && sufficient_lovelace && enough_reserves {
                        return Some(FeeSwitchPool {
                            id: PoolId::try_from(conf.pool_nft).ok()?,
                            reserves_x: TaggedAmount::new(reserves_x),
                            reserves_y: TaggedAmount::new(reserves_y),
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
                            bounds,
                            nonce: 0,
                            stake_part_script_hash: stake_part,
                        });
                    }
                }
                FeeSwitchPoolVer::BiDirV1 => {
                    let conf = FeeSwitchBidirectionalPoolConfig::try_from_pd(pd.clone())?;
                    let liquidity_neg = value.amount_of(conf.asset_lq.into())?;
                    let lov = value.amount_of(Native)?;
                    let reserves_x = value.amount_of(conf.asset_x.into())?;
                    let reserves_y = value.amount_of(conf.asset_y.into())?;
                    let pure_reserves_x = reserves_x - conf.treasury_x;
                    let pure_reserves_y = reserves_y - conf.treasury_y;
                    let non_empty_reserves = pure_reserves_x > 0 && pure_reserves_y > 0;
                    let sufficient_lovelace = conf.asset_x.is_native()
                        || conf.asset_y.is_native()
                        || bounds.min_t2t_lovelace <= lov;
                    let enough_reserves = reserves_x > conf.treasury_x && reserves_y > conf.treasury_y;
                    if non_empty_reserves && sufficient_lovelace && enough_reserves {
                        return Some(FeeSwitchPool {
                            id: PoolId::try_from(conf.pool_nft).ok()?,
                            reserves_x: TaggedAmount::new(reserves_x),
                            reserves_y: TaggedAmount::new(reserves_y),
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
                            bounds,
                            nonce: 0,
                            stake_part_script_hash: stake_part,
                        });
                    }
                }
            }
        };
        None
    }
}

impl AMMOps for FeeSwitchPool {
    fn output_amount(
        &self,
        base_asset: TaggedAssetClass<Base>,
        base_amount: TaggedAmount<Base>,
    ) -> TaggedAmount<Quote> {
        classic_cfmm_output_amount(
            self.asset_x,
            self.asset_y,
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

impl<Ctx> RequiresValidator<Ctx> for FeeSwitchPool
where
    Ctx: Has<DeployedValidator<{ ConstFnPoolFeeSwitch as u8 }>>
        + Has<DeployedValidator<{ ConstFnPoolFeeSwitchV2 as u8 }>>
        + Has<DeployedValidator<{ ConstFnPoolFeeSwitchBiDirFee as u8 }>>,
{
    fn get_validator(&self, ctx: &Ctx) -> DeployedValidatorErased {
        match self.ver {
            FeeSwitchPoolVer::V1 => ctx
                .select::<DeployedValidator<{ ConstFnPoolFeeSwitch as u8 }>>()
                .erased(),
            FeeSwitchPoolVer::V2 => ctx
                .select::<DeployedValidator<{ ConstFnPoolFeeSwitchV2 as u8 }>>()
                .erased(),
            FeeSwitchPoolVer::BiDirV1 => ctx
                .select::<DeployedValidator<{ ConstFnPoolFeeSwitchBiDirFee as u8 }>>()
                .erased(),
        }
    }
}

impl MakerBehavior for FeeSwitchPool {
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
            OnSide::Bid(input) => {
                // A user bid means that they wish to buy the base asset for the quote asset, hence
                // pool reserves of base decreases while reserves of quote increase.
                *quote_reserves += input;
                *base_reserves -= output;
                *quote_treasury += (input * self.treasury_fee.numer()) / self.treasury_fee.denom();
            }
            OnSide::Ask(input) => {
                // User ask is the opposite; sell the base asset for the quote asset.
                *base_reserves += input;
                *quote_reserves -= output;
                *base_treasury += (input * self.treasury_fee.numer()) / self.treasury_fee.denom();
            }
        }
        Next::Succ(self)
    }
}

impl MarketMaker for FeeSwitchPool {
    type U = ExUnits;

    fn static_price(&self) -> SpotPrice {
        let x = self.asset_x.untag();
        let y = self.asset_y.untag();
        let [base, _] = order_canonical(x, y);
        let available_x_reserves = (self.reserves_x - self.treasury_x).untag();
        let available_y_reserves = (self.reserves_y - self.treasury_y).untag();
        if available_x_reserves == available_y_reserves {
            AbsolutePrice::new_unsafe(1, 1).into()
        } else {
            if x == base {
                AbsolutePrice::new_unsafe(available_y_reserves, available_x_reserves).into()
            } else {
                AbsolutePrice::new_unsafe(available_x_reserves, available_y_reserves).into()
            }
        }
    }

    fn real_price(&self, input: OnSide<u64>) -> Option<AbsolutePrice> {
        let x = self.asset_x.untag();
        let y = self.asset_y.untag();
        let [base, quote] = order_canonical(x, y);
        let (base, quote) = match input {
            OnSide::Bid(input) => (
                self.output_amount(TaggedAssetClass::new(quote), TaggedAmount::new(input))
                    .untag(),
                input,
            ),
            OnSide::Ask(input) => (
                input,
                self.output_amount(TaggedAssetClass::new(base), TaggedAmount::new(input))
                    .untag(),
            ),
        };
        AbsolutePrice::new(quote, base)
    }

    fn quality(&self) -> PoolQuality {
        PoolQuality::from(self.liquidity.untag())
    }

    fn marginal_cost_hint(&self) -> Self::U {
        self.marginal_cost
    }

    fn liquidity(&self) -> AbsoluteReserves {
        let x = self.asset_x.untag();
        let y = self.asset_y.untag();
        let [base, _] = order_canonical(x, y);
        if base == x {
            AbsoluteReserves {
                base: self.reserves_x.untag(),
                quote: self.reserves_y.untag(),
            }
        } else {
            AbsoluteReserves {
                base: self.reserves_y.untag(),
                quote: self.reserves_x.untag(),
            }
        }
    }

    fn available_liquidity_on_side(&self, worst_price: OnSide<AbsolutePrice>) -> Option<AvailableLiquidity> {
        let sqrt_degree = BigNumber::from(0.5);

        let [base, _] = order_canonical(self.asset_x.untag(), self.asset_y.untag());

        let tradable_x_reserves_int = (self.reserves_x - self.treasury_x).untag();
        let tradable_x_reserves = BigNumber::from(tradable_x_reserves_int as f64);
        let tradable_y_reserves_int = (self.reserves_y - self.treasury_y).untag();
        let tradable_y_reserves = BigNumber::from(tradable_y_reserves_int as f64);
        let raw_fee_x = self.lp_fee_x.checked_sub(&self.treasury_fee)?;
        let fee_x = BigNumber::from(raw_fee_x.to_f64()?);
        let raw_fee_y = self.lp_fee_y.checked_sub(&self.treasury_fee)?;
        let fee_y = BigNumber::from(raw_fee_y.to_f64()?);
        let bid_price = BigNumber::from(*worst_price.unwrap().denom() as f64)
            / BigNumber::from(*worst_price.unwrap().numer() as f64);
        let ask_price = BigNumber::from(*worst_price.unwrap().numer() as f64)
            / BigNumber::from(*worst_price.unwrap().denom() as f64);

        let (
            tradable_reserves_quote_int,
            tradable_reserves_base,
            tradable_reserves_quote,
            total_fee_mult,
            avg_price,
        ) = match worst_price {
            OnSide::Bid(_) if base == self.asset_x.untag() => (
                tradable_x_reserves_int,
                tradable_y_reserves,
                tradable_x_reserves,
                fee_y,
                bid_price,
            ),
            OnSide::Bid(_) => (
                tradable_y_reserves_int,
                tradable_x_reserves,
                tradable_y_reserves,
                fee_x,
                bid_price,
            ),
            OnSide::Ask(_) if base == self.asset_x.untag() => (
                tradable_y_reserves_int,
                tradable_x_reserves,
                tradable_y_reserves,
                fee_x,
                ask_price,
            ),
            OnSide::Ask(_) => (
                tradable_x_reserves_int,
                tradable_y_reserves,
                tradable_x_reserves,
                fee_y,
                ask_price,
            ),
        };

        let lq_balance = (tradable_reserves_base.clone() * tradable_reserves_quote.clone()).pow(&sqrt_degree);

        let p1 = (avg_price.div(total_fee_mult.clone()) * lq_balance.clone()
            / tradable_reserves_quote.clone())
        .pow(&(BigNumber::from(2)));
        let p1_sqrt = p1.clone().pow(&sqrt_degree);
        let x1 = lq_balance.clone() / p1_sqrt.clone();
        let y1 = lq_balance.clone() * p1_sqrt.clone();

        let input_amount = (x1.clone() - tradable_reserves_base.clone()) / total_fee_mult;
        let output_amount = tradable_reserves_quote - y1.clone();

        let input_amount_val = <u64>::try_from(input_amount.value.to_int().value()).ok()?;
        let output_amount_val = <u64>::try_from(output_amount.value.to_int().value()).ok()?;
        let remain_quote = tradable_reserves_quote_int.checked_sub(output_amount_val)?;

        let capped_output_amount = match UNTOUCHABLE_LOVELACE_AMOUNT.checked_sub(remain_quote) {
            Some(_) => tradable_reserves_quote_int.checked_sub(UNTOUCHABLE_LOVELACE_AMOUNT)?,
            None => output_amount_val,
        };

        Some(AvailableLiquidity {
            input: input_amount_val,
            output: capped_output_amount,
        })
    }

    fn estimated_trade(&self, input: OnSide<u64>) -> Option<AvailableLiquidity> {
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
        Some(AvailableLiquidity {
            input: input.unwrap(),
            output,
        })
    }

    fn is_active(&self) -> bool {
        let lq_bound = (self.reserves_x.untag() * 2) >= self.lq_lower_bound.untag();
        let native_bound = if self.asset_x.is_native() {
            self.reserves_x.untag() >= self.bounds.min_n2t_lovelace
        } else if self.asset_y.is_native() {
            self.reserves_y.untag() >= self.bounds.min_n2t_lovelace
        } else {
            true
        };
        lq_bound && native_bound
    }
}

impl Stable for FeeSwitchPool {
    type StableId = PoolId;

    fn stable_id(&self) -> Self::StableId {
        self.id
    }

    fn is_quasi_permanent(&self) -> bool {
        true
    }
}

impl IntoLedger<TransactionOutput, ImmutablePoolUtxo> for FeeSwitchPool {
    fn into_ledger(self, mut immut_pool: ImmutablePoolUtxo) -> TransactionOutput {
        let mut ma = MultiAsset::new();
        let coins = if self.asset_x.is_native() {
            let Token(policy, name) = self.asset_y.untag().into_token().unwrap();
            ma.set(policy, name.into(), self.reserves_y.untag());
            self.reserves_x.untag()
        } else if self.asset_y.is_native() {
            let Token(policy, name) = self.asset_x.untag().into_token().unwrap();
            ma.set(policy, name.into(), self.reserves_x.untag());
            self.reserves_y.untag()
        } else {
            let Token(policy_x, name_x) = self.asset_x.untag().into_token().unwrap();
            ma.set(policy_x, name_x.into(), self.reserves_x.untag());
            let Token(policy_y, name_y) = self.asset_y.untag().into_token().unwrap();
            ma.set(policy_y, name_y.into(), self.reserves_y.untag());
            immut_pool.value
        };
        let Token(policy_lq, name_lq) = self.asset_lq.untag().into_token().unwrap();
        let Token(nft_lq, name_nft) = self.id.into();
        ma.set(policy_lq, name_lq.into(), MAX_LQ_CAP - self.liquidity.untag());
        ma.set(nft_lq, name_nft.into(), 1);

        if let Some(DatumOption::Datum { datum, .. }) = &mut immut_pool.datum_option {
            unsafe_update_pd_fee_switch(datum, self.treasury_x.untag(), self.treasury_y.untag());
        }

        if let Some(stake_part_script_hash) = self.stake_part_script_hash {
            immut_pool
                .address
                .update_staking_cred(Credential::new_script(stake_part_script_hash))
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

mod tests {
    use crate::data::cfmm_pool::fee_switch_pool::FeeSwitchPoolConfig;
    use cml_chain::plutus::PlutusData;
    use cml_chain::Deserialize;
    use spectrum_cardano_lib::types::TryFromPData;

    const DATUM_SAMPLE: &str =
        "d8799fd8799f581c6aaa652b39f5723afc85bba38401a4cbfd5b2f7aa3771504257ac8a74d74657374425f4144415f4e4654ffd8799f4040ffd8799f581c4b3459fd18a1dbabe207cd19c9951a9fac9f5c0f9c384e3d97efba26457465737442ffd8799f581c635f44ae5df86be9e80fd0c57a5ec699a146d9d9034516ffd72febef4c74657374425f4144415f4c51ff19270b010000801b00000002540be400581c2618e94cdb06792f05ae9b1ec78b0231f4b7f4215b1b4cf52e6342deff";

    #[test]
    fn parse_fee_switch_datum_mainnet() {
        let pd = PlutusData::from_cbor_bytes(&*hex::decode(DATUM_SAMPLE).unwrap()).unwrap();
        let maybe_conf = FeeSwitchPoolConfig::try_from_pd(pd);
        assert!(maybe_conf.is_some())
    }
}
