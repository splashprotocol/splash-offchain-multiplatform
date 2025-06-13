use crate::constants::{FEE_DEN, LEGACY_FEE_NUM_MULTIPLIER, MAX_LQ_CAP};
use crate::data::cfmm_pool::{AMMOps, ClassicPoolVer, LegacyCFMMPoolConfig};
use crate::data::order::{Base, Quote};
use crate::data::pair::order_canonical;
use crate::data::pool::{ImmutablePoolUtxo, Lq, PoolValidation, Rx, Ry};
use crate::data::PoolId;
use crate::deployment::ProtocolValidator::{ConstFnPoolV1, ConstFnPoolV2};
use crate::deployment::{DeployedScriptInfo, DeployedValidator, DeployedValidatorErased, RequiresValidator};
use crate::pool_math::cfmm_math::{
    classic_cfmm_output_amount, classic_cfmm_reward_lp, classic_cfmm_shares_amount,
    UNTOUCHABLE_LOVELACE_AMOUNT,
};
use bignumber::BigNumber;
use bloom_offchain::execution_engine::liquidity_book::market_maker::{
    AbsoluteReserves, AvailableLiquidity, MarketMaker, PoolQuality, SpotPrice,
};
use bloom_offchain::execution_engine::liquidity_book::side::OnSide;
use bloom_offchain::execution_engine::liquidity_book::types::AbsolutePrice;
use cml_chain::assets::MultiAsset;
use cml_chain::certs::{Credential, StakeCredential};
use cml_chain::transaction::{ConwayFormatTxOut, TransactionOutput};
use cml_chain::Value;
use cml_crypto::ScriptHash;
use num_rational::Ratio;
use num_traits::ToPrimitive;
use spectrum_cardano_lib::address::AddressExtension;
use spectrum_cardano_lib::ex_units::ExUnits;
use spectrum_cardano_lib::plutus_data::DatumExtension;
use spectrum_cardano_lib::transaction::TransactionOutputExtension;
use spectrum_cardano_lib::types::TryFromPData;
use spectrum_cardano_lib::value::ValueExtension;
use spectrum_cardano_lib::{TaggedAmount, TaggedAssetClass, Token};
use spectrum_offchain::domain::Has;
use spectrum_offchain::ledger::{IntoLedger, TryFromLedger};
use std::fmt::{Display, Formatter};
use std::ops::Div;

/// Represents a classic CFMM (Constant Function Market Maker) pool implementation.
///
/// This version does not include any advanced features such as royalties or treasury fees.
#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub struct ClassicPool {
    pub id: PoolId,
    pub reserves_x: TaggedAmount<Rx>,
    pub reserves_y: TaggedAmount<Ry>,
    pub liquidity: TaggedAmount<Lq>,
    pub asset_x: TaggedAssetClass<Rx>,
    pub asset_y: TaggedAssetClass<Ry>,
    pub asset_lq: TaggedAssetClass<Lq>,
    pub lp_fee: Ratio<u64>,
    pub lq_lower_bound: TaggedAmount<Rx>,
    pub ver: ClassicPoolVer,
    pub marginal_cost: ExUnits,
    pub bounds: PoolValidation,
    pub stake_part_script_hash: Option<ScriptHash>,
}

impl Display for ClassicPool {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(&*format!(
            "ClassicPool(id: {}, static_price: {}, rx: {}, ry: {})",
            self.id,
            self.static_price(),
            self.reserves_x,
            self.reserves_y,
        ))
    }
}

impl<Ctx> TryFromLedger<TransactionOutput, Ctx> for ClassicPool
where
    Ctx: Has<DeployedScriptInfo<{ ConstFnPoolV1 as u8 }>>
        + Has<DeployedScriptInfo<{ ConstFnPoolV2 as u8 }>>
        + Has<PoolValidation>,
{
    fn try_from_ledger(repr: &TransactionOutput, ctx: &Ctx) -> Option<Self> {
        if let Some(pool_ver) = ClassicPoolVer::try_from_address(repr.address(), ctx) {
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
                ClassicPoolVer::V1 => {
                    ctx.select::<DeployedScriptInfo<{ ConstFnPoolV1 as u8 }>>()
                        .marginal_cost
                }
                ClassicPoolVer::V2 => {
                    ctx.select::<DeployedScriptInfo<{ ConstFnPoolV2 as u8 }>>()
                        .marginal_cost
                }
            };
            let conf = LegacyCFMMPoolConfig::try_from_pd(pd.clone())?;
            let liquidity_neg = value.amount_of(conf.asset_lq.into())?;
            return Some(ClassicPool {
                id: PoolId::try_from(conf.pool_nft).ok()?,
                reserves_x: TaggedAmount::new(value.amount_of(conf.asset_x.into())?),
                reserves_y: TaggedAmount::new(value.amount_of(conf.asset_y.into())?),
                liquidity: TaggedAmount::new(MAX_LQ_CAP - liquidity_neg),
                asset_x: conf.asset_x,
                asset_y: conf.asset_y,
                asset_lq: conf.asset_lq,
                // legacy lp fee den = 1000
                // new lp fee den = 100000
                lp_fee: Ratio::new_raw(conf.lp_fee_num * LEGACY_FEE_NUM_MULTIPLIER, FEE_DEN),
                lq_lower_bound: conf.lq_lower_bound,
                ver: pool_ver,
                marginal_cost,
                bounds,
                stake_part_script_hash: stake_part,
            });
        };
        None
    }
}

impl AMMOps for ClassicPool {
    fn output_amount(
        &self,
        base_asset: TaggedAssetClass<Base>,
        base_amount: TaggedAmount<Base>,
    ) -> TaggedAmount<Quote> {
        classic_cfmm_output_amount(
            self.asset_x,
            self.asset_y,
            self.reserves_x,
            self.reserves_y,
            base_asset,
            base_amount,
            self.lp_fee,
            self.lp_fee,
        )
    }

    fn reward_lp(
        &self,
        in_x_amount: u64,
        in_y_amount: u64,
    ) -> Option<(TaggedAmount<Lq>, TaggedAmount<Rx>, TaggedAmount<Ry>)> {
        classic_cfmm_reward_lp(
            self.reserves_x,
            self.reserves_y,
            self.liquidity,
            in_x_amount,
            in_y_amount,
        )
    }

    fn shares_amount(&self, burned_lq: TaggedAmount<Lq>) -> Option<(TaggedAmount<Rx>, TaggedAmount<Ry>)> {
        classic_cfmm_shares_amount(self.reserves_x, self.reserves_y, self.liquidity, burned_lq)
    }
}

impl<Ctx> RequiresValidator<Ctx> for ClassicPool
where
    Ctx: Has<DeployedValidator<{ crate::deployment::ProtocolValidator::ConstFnPoolV1 as u8 }>>
        + Has<DeployedValidator<{ crate::deployment::ProtocolValidator::ConstFnPoolV2 as u8 }>>,
{
    fn get_validator(&self, ctx: &Ctx) -> DeployedValidatorErased {
        match self.ver {
            ClassicPoolVer::V1 => ctx
                .select::<DeployedValidator<{ crate::deployment::ProtocolValidator::ConstFnPoolV1 as u8 }>>()
                .erased(),
            ClassicPoolVer::V2 => ctx
                .select::<DeployedValidator<{ crate::deployment::ProtocolValidator::ConstFnPoolV2 as u8 }>>()
                .erased(),
        }
    }
}

impl MarketMaker for ClassicPool {
    type U = ExUnits;

    fn static_price(&self) -> SpotPrice {
        let x = self.asset_x.untag();
        let y = self.asset_y.untag();
        let [base, _] = order_canonical(x, y);
        let available_x_reserves = self.reserves_x.untag();
        let available_y_reserves = self.reserves_y.untag();
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

        let tradable_x_reserves_int = self.reserves_x.untag();
        let tradable_x_reserves = BigNumber::from(tradable_x_reserves_int as f64);
        let tradable_y_reserves_int = self.reserves_y.untag();
        let tradable_y_reserves = BigNumber::from(tradable_y_reserves_int as f64);
        let raw_fee_x = self.lp_fee;
        let fee_x = BigNumber::from(raw_fee_x.to_f64()?);
        let raw_fee_y = self.lp_fee;
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
}

impl IntoLedger<TransactionOutput, ImmutablePoolUtxo> for ClassicPool {
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
