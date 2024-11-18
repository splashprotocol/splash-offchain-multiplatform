use std::fmt::Debug;
use std::ops::Div;
use std::ops::Mul;

use bignumber::BigNumber;
use cml_chain::address::Address;
use cml_chain::assets::MultiAsset;
use cml_chain::certs::StakeCredential;
use cml_chain::plutus::utils::ConstrPlutusDataEncoding;
use cml_chain::plutus::{ConstrPlutusData, PlutusData};
use cml_chain::transaction::{ConwayFormatTxOut, DatumOption, TransactionOutput};
use cml_chain::utils::BigInteger;
use cml_chain::Value;
use cml_core::serialization::LenEncoding::{Canonical, Indefinite};
use num_bigint::BigInt;
use num_rational::{BigRational, Ratio};
use num_traits::{CheckedAdd, CheckedDiv, CheckedMul, CheckedSub, FromPrimitive, One};
use num_traits::{ToPrimitive, Zero};
use primitive_types::U512;
use void::Void;

use bloom_offchain::execution_engine::liquidity_book::core::Next;
use bloom_offchain::execution_engine::liquidity_book::market_maker::{
    AbsoluteReserves, MakerBehavior, MarketMaker, PoolQuality, SpotPrice,
};
use bloom_offchain::execution_engine::liquidity_book::market_maker::{
    AvailableLiquidity, FullPriceDerivative,
};
use bloom_offchain::execution_engine::liquidity_book::side::{OnSide, Side, SwapAssetSide};
use bloom_offchain::execution_engine::liquidity_book::types::AbsolutePrice;
use spectrum_cardano_lib::ex_units::ExUnits;
use spectrum_cardano_lib::plutus_data::{ConstrPlutusDataExtension, DatumExtension};
use spectrum_cardano_lib::plutus_data::{IntoPlutusData, PlutusDataExtension};
use spectrum_cardano_lib::transaction::TransactionOutputExtension;
use spectrum_cardano_lib::types::TryFromPData;
use spectrum_cardano_lib::value::ValueExtension;
use spectrum_cardano_lib::AssetClass::Native;
use spectrum_cardano_lib::{TaggedAmount, TaggedAssetClass, Token};
use spectrum_offchain::data::{Has, Stable};
use spectrum_offchain::ledger::{IntoLedger, TryFromLedger};

use crate::constants::{ADA_WEIGHT, FEE_DEN, MAX_LQ_CAP, TOKEN_WEIGHT, WEIGHT_FEE_DEN};
use crate::data::cfmm_pool::AMMOps;
use crate::data::deposit::ClassicalOnChainDeposit;
use crate::data::operation_output::{DepositOutput, RedeemOutput};
use crate::data::order::{Base, PoolNft, Quote};
use crate::data::pair::order_canonical;
use crate::data::pool::{
    ApplyOrder, ApplyOrderError, CFMMPoolAction, ImmutablePoolUtxo, Lq, PoolAssetMapping, PoolValidation, Rx,
    Ry,
};
use crate::data::redeem::ClassicalOnChainRedeem;
use crate::data::PoolId;
use crate::deployment::ProtocolValidator::{BalanceFnPoolV1, BalanceFnPoolV2};
use crate::deployment::{DeployedScriptInfo, DeployedValidator, DeployedValidatorErased, RequiresValidator};
use crate::pool_math::balance_math::{
    balance_cfmm_output_amount, price_estimation_error, simple_estimate_inp_by_spot_price,
    spot_price_estimation_error,
};
use crate::pool_math::cfmm_math::{classic_cfmm_reward_lp, classic_cfmm_shares_amount};

#[derive(Debug)]
pub struct BalancePoolConfig {
    pub pool_nft: TaggedAssetClass<PoolNft>,
    pub asset_x: TaggedAssetClass<Rx>,
    pub asset_y: TaggedAssetClass<Ry>,
    pub asset_lq: TaggedAssetClass<Lq>,
    pub lp_fee_num: u64,
    pub treasury_fee_num: u64,
    pub treasury_x: u64,
    pub treasury_y: u64,
}

impl TryFromPData for BalancePoolConfig {
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
        })
    }
}

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub enum BalancePoolVer {
    V1,
    V2,
}

impl BalancePoolVer {
    pub fn try_from_address<Ctx>(pool_addr: &Address, ctx: &Ctx) -> Option<BalancePoolVer>
    where
        Ctx: Has<DeployedScriptInfo<{ BalanceFnPoolV1 as u8 }>>,
        Ctx: Has<DeployedScriptInfo<{ BalanceFnPoolV2 as u8 }>>,
    {
        let maybe_hash = pool_addr.payment_cred().and_then(|c| match c {
            StakeCredential::PubKey { .. } => None,
            StakeCredential::Script { hash, .. } => Some(hash),
        });
        if let Some(this_hash) = maybe_hash {
            if ctx
                .select::<DeployedScriptInfo<{ BalanceFnPoolV1 as u8 }>>()
                .script_hash
                == *this_hash
            {
                return Some(BalancePoolVer::V1);
            } else if ctx
                .select::<DeployedScriptInfo<{ BalanceFnPoolV2 as u8 }>>()
                .script_hash
                == *this_hash
            {
                return Some(BalancePoolVer::V2);
            }
        };
        None
    }
}

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub struct BalancePool {
    pub id: PoolId,
    pub reserves_x: TaggedAmount<Rx>,
    pub weight_x: u64,
    pub reserves_y: TaggedAmount<Ry>,
    pub weight_y: u64,
    pub liquidity: TaggedAmount<Lq>,
    pub asset_x: TaggedAssetClass<Rx>,
    pub asset_y: TaggedAssetClass<Ry>,
    pub asset_lq: TaggedAssetClass<Lq>,
    pub lp_fee_x: Ratio<u64>,
    pub lp_fee_y: Ratio<u64>,
    pub treasury_fee: Ratio<u64>,
    pub treasury_x: TaggedAmount<Rx>,
    pub treasury_y: TaggedAmount<Ry>,
    pub ver: BalancePoolVer,
    /// How many execution units pool invokation costs.
    pub marginal_cost: ExUnits,
    pub min_pool_lovelace: u64,
}

impl BalancePool {
    fn calculate_swap_invariant(
        base_reserves: u64,
        base_delta: u64,
        base_fee: Ratio<u64>,
        base_treasury: u64,
        base_weight: u64,
        quote_reserves: u64,
        quote_delta: u64,
        quote_treasury: u64,
        quote_weight: u64,
        treasury_fee: Ratio<u64>,
    ) -> U512 {
        let base_delta_in_invariant =
            (base_delta * (base_fee.numer() - treasury_fee.numer())) / base_fee.denom();

        let base_part = BigNumber::from((base_reserves - base_treasury + base_delta_in_invariant) as f64)
            .pow(&BigNumber::from(base_weight as f64));

        let quote_part = BigNumber::from((quote_reserves - quote_treasury - quote_delta) as f64)
            .pow(&BigNumber::from(quote_weight as f64));

        U512::from_str_radix(base_part.mul(quote_part).to_string().as_str(), 10).unwrap()
    }

    fn calculate_deposit_redeem_invariant(&self) -> U512 {
        let x_part = BigNumber::from((self.reserves_x.untag() - self.treasury_x.untag()) as f64)
            .pow(&BigNumber::from(self.weight_x as f64));

        let y_part = BigNumber::from((self.reserves_y.untag() - self.treasury_y.untag()) as f64)
            .pow(&BigNumber::from(self.weight_y as f64));

        U512::from_str_radix(x_part.mul(y_part).to_string().as_str(), 10).unwrap()
    }

    pub fn get_asset_deltas(&self, side: Side) -> PoolAssetMapping {
        let x = self.asset_x.untag();
        let y = self.asset_y.untag();
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

    // [gx, tx, gy, ty]
    fn create_redeemer(pool_action: CFMMPoolAction, pool_idx: u64) -> PlutusData {
        /*
          Original structure of pool redeemer
            [ "action" ':= BalancePoolAction
            , "selfIx" ':= PInteger
            ]
        */

        let action_plutus_data = pool_action.to_plutus_data();
        let self_ix_pd = PlutusData::Integer(BigInteger::from(pool_idx));

        PlutusData::ConstrPlutusData(ConstrPlutusData {
            alternative: 0,
            fields: Vec::from([action_plutus_data, self_ix_pd]),
            encodings: Some(ConstrPlutusDataEncoding {
                len_encoding: Canonical,
                tag_encoding: Some(cbor_event::Sz::One),
                alternative_encoding: None,
                fields_encoding: Indefinite,
                prefer_compact: true,
            }),
        })
    }
}

impl<Ctx> TryFromLedger<TransactionOutput, Ctx> for BalancePool
where
    Ctx: Has<DeployedScriptInfo<{ BalanceFnPoolV1 as u8 }>>
        + Has<DeployedScriptInfo<{ BalanceFnPoolV2 as u8 }>>
        + Has<PoolValidation>,
{
    fn try_from_ledger(repr: &TransactionOutput, ctx: &Ctx) -> Option<Self> {
        if let Some(pool_ver) = BalancePoolVer::try_from_address(repr.address(), ctx) {
            let value = repr.value();
            let pd = repr.datum().clone()?.into_pd()?;
            let conf = BalancePoolConfig::try_from_pd(pd.clone())?;
            let liquidity_neg = value.amount_of(conf.asset_lq.into())?;
            let bounds = ctx.select::<PoolValidation>();
            let lov = value.amount_of(Native)?;
            if conf.asset_x.is_native() || conf.asset_y.is_native() || bounds.min_t2t_lovelace <= lov {
                return Some(BalancePool {
                    id: PoolId::try_from(conf.pool_nft).ok()?,
                    reserves_x: TaggedAmount::new(value.amount_of(conf.asset_x.into())?),
                    weight_x: ADA_WEIGHT,
                    reserves_y: TaggedAmount::new(value.amount_of(conf.asset_y.into())?),
                    weight_y: TOKEN_WEIGHT,
                    liquidity: TaggedAmount::new(MAX_LQ_CAP - liquidity_neg),
                    asset_x: conf.asset_x,
                    asset_y: conf.asset_y,
                    asset_lq: conf.asset_lq,
                    lp_fee_x: Ratio::new_raw(conf.lp_fee_num, FEE_DEN),
                    lp_fee_y: Ratio::new_raw(conf.lp_fee_num, FEE_DEN),
                    treasury_fee: Ratio::new_raw(conf.treasury_fee_num, FEE_DEN),
                    treasury_x: TaggedAmount::new(conf.treasury_x),
                    treasury_y: TaggedAmount::new(conf.treasury_y),
                    ver: pool_ver,
                    marginal_cost: ctx
                        .select::<DeployedScriptInfo<{ BalanceFnPoolV1 as u8 }>>()
                        .marginal_cost,
                    min_pool_lovelace: bounds.min_n2t_lovelace,
                });
            }
        }
        None
    }
}

impl IntoLedger<TransactionOutput, ImmutablePoolUtxo> for BalancePool {
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
            unsafe_update_datum(datum, self.treasury_x.untag(), self.treasury_y.untag());
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

pub fn unsafe_update_datum(data: &mut PlutusData, treasury_x: u64, treasury_y: u64) {
    let cpd = data.get_constr_pd_mut().unwrap();
    cpd.set_field(6, treasury_x.into_pd());
    cpd.set_field(7, treasury_y.into_pd());
}

impl Stable for BalancePool {
    type StableId = PoolId;
    fn stable_id(&self) -> Self::StableId {
        self.id
    }
    fn is_quasi_permanent(&self) -> bool {
        true
    }
}

impl<Ctx> RequiresValidator<Ctx> for BalancePool
where
    Ctx:
        Has<DeployedValidator<{ BalanceFnPoolV1 as u8 }>> + Has<DeployedValidator<{ BalanceFnPoolV2 as u8 }>>,
{
    fn get_validator(&self, ctx: &Ctx) -> DeployedValidatorErased {
        match self.ver {
            BalancePoolVer::V1 => ctx
                .select::<DeployedValidator<{ BalanceFnPoolV1 as u8 }>>()
                .erased(),
            BalancePoolVer::V2 => ctx
                .select::<DeployedValidator<{ BalanceFnPoolV2 as u8 }>>()
                .erased(),
        }
    }
}

pub struct BalancePoolRedeemer {
    pub pool_input_index: u64,
    pub action: CFMMPoolAction,
    pub new_pool_state: BalancePool,
    pub prev_pool_state: BalancePool,
}

impl BalancePoolRedeemer {
    pub fn to_plutus_data(self) -> PlutusData {
        BalancePool::create_redeemer(self.action, self.pool_input_index)
    }
}

impl AMMOps for BalancePool {
    fn output_amount(
        &self,
        base_asset: TaggedAssetClass<Base>,
        base_amount: TaggedAmount<Base>,
    ) -> TaggedAmount<Quote> {
        balance_cfmm_output_amount(
            self.asset_x,
            self.reserves_x - self.treasury_x,
            self.weight_x,
            self.reserves_y - self.treasury_y,
            self.weight_y,
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
        // Balance pool reward lp calculation is the same as for cfmm pool,
        // but we should "recalculate" change_x, change_y based on unlocked_lq
        let (unlocked_lq, _change_x, _change_y) = classic_cfmm_reward_lp(
            self.reserves_x - self.treasury_x,
            self.reserves_y - self.treasury_y,
            self.liquidity,
            in_x_amount,
            in_y_amount,
        )?;

        let x_to_deposit = (unlocked_lq.untag() as u128
            * (self.reserves_x.untag() - self.treasury_x.untag()) as u128)
            / (self.liquidity.untag() as u128);
        let y_to_deposit_bn = (unlocked_lq.untag() as u128
            * (self.reserves_y.untag() - self.treasury_y.untag()) as u128)
            / (self.liquidity.untag() as u128);

        Some((
            unlocked_lq,
            TaggedAmount::new(in_x_amount - x_to_deposit as u64),
            TaggedAmount::new(in_y_amount - y_to_deposit_bn as u64),
        ))
    }

    fn shares_amount(&self, burned_lq: TaggedAmount<Lq>) -> Option<(TaggedAmount<Rx>, TaggedAmount<Ry>)> {
        // Balance pool shares amount calculation is the same as for cfmm pool
        classic_cfmm_shares_amount(
            self.reserves_x - self.treasury_x,
            self.reserves_y - self.treasury_y,
            self.liquidity,
            burned_lq,
        )
    }
}

impl MakerBehavior for BalancePool {
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
                let treasury_fee = BigInt::from(input)
                    .mul(BigInt::from(*self.treasury_fee.numer()))
                    .div(BigInt::from(*self.treasury_fee.denom()));
                *quote_reserves += input;
                *base_reserves -= output;
                *quote_treasury += treasury_fee.to_u64().unwrap();
            }
            OnSide::Ask(input) => {
                // User ask is the opposite; sell the base asset for the quote asset.
                let treasury_fee = BigInt::from(input)
                    .mul(BigInt::from(*self.treasury_fee.numer()))
                    .div(BigInt::from(*self.treasury_fee.denom()));
                *base_reserves += input;
                *quote_reserves -= output;
                *base_treasury += treasury_fee.to_u64().unwrap();
            }
        }
        Next::Succ(self)
    }
}

impl MarketMaker for BalancePool {
    type U = ExUnits;
    fn static_price(&self) -> SpotPrice {
        let x = self.asset_x.untag();
        let y = self.asset_y.untag();
        let [base, _] = order_canonical(x, y);
        let available_x_reserves = (self.reserves_x - self.treasury_x).untag();
        let available_y_reserves = (self.reserves_y - self.treasury_y).untag();
        let available_x_reserves_big = BigInt::from(available_x_reserves);
        let available_y_reserves_big = BigInt::from(available_y_reserves);
        let fee_den_big = BigInt::from(WEIGHT_FEE_DEN);
        let weight_x_big = BigInt::from(self.weight_x);
        let weight_y_big = BigInt::from(self.weight_y);
        let price_num = available_y_reserves_big.div(weight_y_big).to_u64().unwrap();
        let price_denom = available_x_reserves_big.div(weight_x_big).to_u64().unwrap();

        if x == base {
            AbsolutePrice::new_unsafe(price_num, price_denom).into()
        } else {
            AbsolutePrice::new_unsafe(price_denom, price_num).into()
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
        if self.asset_x.is_native() {
            self.reserves_x.untag() >= self.min_pool_lovelace
        } else if self.asset_y.is_native() {
            self.reserves_y.untag() >= self.min_pool_lovelace
        } else {
            true
        }
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
    fn available_liquidity_by_order_price(
        &self,
        worst_price: OnSide<AbsolutePrice>,
    ) -> Option<AvailableLiquidity> {
        let x_asset = self.asset_x.untag();
        let [base, _] = order_canonical(self.asset_x.untag(), self.asset_y.untag());
        let x_is_base = x_asset == base;

        let x_reserves = BigInt::from((self.reserves_x - self.treasury_x).untag());
        let y_reserves = BigInt::from((self.reserves_y - self.treasury_y).untag());
        let x_fee = self.lp_fee_x - self.treasury_fee;
        let y_fee = self.lp_fee_y - self.treasury_fee;

        let spot_price_orig_f64 = self.static_price().unwrap().to_f64()?;

        let worst_price_value = worst_price.unwrap();
        let price_num = BigInt::try_from(*worst_price_value.numer()).ok()?;
        let price_denom = BigInt::try_from(*worst_price_value.denom()).ok()?;
        // Side of the operation:
        let (side,) = match worst_price {
            OnSide::Ask(_) => (Side::Ask,),
            OnSide::Bid(_) => (Side::Bid,),
        };
        let (
            in_balance,
            out_balance,
            total_fee_mult_num,
            total_fee_mult_denom,
            spot_price_f64,
            avg_price_num_original,
            avg_price_denom_original,
            w_in,
            w_out,
        ) = if (side == Side::Ask && x_is_base) || (side == Side::Bid && !x_is_base) {
            (
                x_reserves,
                y_reserves,
                BigInt::from(*x_fee.numer()),
                BigInt::from(*x_fee.denom()),
                spot_price_orig_f64,
                price_num,
                price_denom,
                self.weight_x as u32,
                self.weight_y as u32,
            )
        } else {
            (
                y_reserves,
                x_reserves,
                BigInt::from(*y_fee.numer()),
                BigInt::from(*y_fee.denom()),
                spot_price_orig_f64.recip(),
                price_denom,
                price_num,
                self.weight_y as u32,
                self.weight_x as u32,
            )
        };
        // Calculate price derivatives to estimate required inp & out:
        let derivative_in = self.full_price_derivative(side, SwapAssetSide::Input)?.0;
        let derivative_in_num = BigInt::try_from(*derivative_in.numer()).ok()?;
        let derivative_in_denom = BigInt::try_from(*derivative_in.denom()).ok()?;

        let avg_price_num = avg_price_num_original.checked_mul(&total_fee_mult_denom)?;
        let avg_price_denom = avg_price_denom_original.checked_mul(&total_fee_mult_num)?;
        let avg_price_f64 = BigRational::new(avg_price_num.clone(), avg_price_denom.clone()).to_f64()?;
        // println!("avg_price_f64: {}", avg_price_f64);
        let scale_factor = 4f64;
        let target_price_delta_f64 = scale_factor * (spot_price_f64 - avg_price_f64);
        let target_price_delta = BigRational::from_f64(target_price_delta_f64)?;
        let b = total_fee_mult_num
            .checked_mul(&target_price_delta.numer())?
            .checked_mul(&derivative_in_denom)?
            .checked_div(
                &total_fee_mult_denom
                    .checked_mul(&target_price_delta.denom())?
                    .checked_mul(&derivative_in_num)?,
            )?;
        // println!("Est in: {}", b.to_u64().unwrap());
        let f_b = price_estimation_error(
            &in_balance,
            &out_balance,
            &b,
            &w_in,
            &w_out,
            &avg_price_num,
            &avg_price_denom,
        )?;
        let mut x_n_1 = BigInt::from(2);
        let max_iter = 25;
        let mut i = 0;
        loop {
            i += 1;
            let f_x_n_1 = price_estimation_error(
                &in_balance,
                &out_balance,
                &x_n_1,
                &w_in,
                &w_out,
                &avg_price_num,
                &avg_price_denom,
            )?;

            let x_n = x_n_1.to_f64().unwrap()
                - f_x_n_1.to_f64().unwrap() * (b.to_f64().unwrap() - x_n_1.to_f64().unwrap())
                    / (f_b.to_f64().unwrap() - f_x_n_1.to_f64().unwrap());
            // let x_n = x_n_1.to_f64().unwrap() - f_x_n_1.to_f64().unwrap() * (b.to_f64().unwrap() - x_n_1.to_f64().unwrap()) / (f_b.to_f64().unwrap() - f_x_n_1.to_f64().unwrap());
            // println!("f_b {}", f_b.to_f64().unwrap());
            // println!("f_x_n_1 {}", f_x_n_1.to_f64().unwrap());
            // println!(
            //     "x_n_pure {}",
            //     x_n
            // );
            // println!(
            //     "x_n {}",
            //     x_n * total_fee_mult_denom.to_f64().unwrap() / total_fee_mult_num.to_f64().unwrap()
            // );
            // println!("b {}", b);
            // println!("Err: {}", (x_n_1.to_f64()? - x_n).abs());
            if (x_n_1.to_f64()? - x_n).abs() < 1f64 || i >= max_iter {
                x_n_1 = BigInt::from_f64(x_n)
                    .unwrap()
                    .checked_mul(&BigInt::from(999))?
                    .checked_div(&BigInt::from(1000))?;
                break;
            }
            x_n_1 = BigInt::from_f64(x_n).unwrap();
        }
        let est_input_amount = x_n_1
            .checked_mul(&total_fee_mult_denom)?
            .checked_div(&total_fee_mult_num)?;
        // println!("x_n_1: {}", x_n_1.to_u64().unwrap());
        // println!("est_input_amount: {}", est_input_amount.to_u64().unwrap());
        let input_for_estimation = match side {
            Side::Ask => OnSide::Ask(est_input_amount.to_u64()?),
            Side::Bid => OnSide::Bid(est_input_amount.to_u64()?),
        };
        self.estimated_trade(input_for_estimation)
    }
    fn available_liquidity_by_spot_price(&self, final_spot_price: SpotPrice) -> Option<AvailableLiquidity> {
        // Canonical ordering:
        let (x_asset, y_asset) = (self.asset_x.untag(), self.asset_y.untag());
        let [base, _] = order_canonical(x_asset, y_asset);
        let x_is_base = x_asset == base;
        // Side of the operation:
        let side = if final_spot_price.unwrap().to_f64() < self.static_price().unwrap().to_f64() {
            Side::Ask
        } else {
            Side::Bid
        };
        // Pool params according to the side:
        let x_reserves = BigInt::from((self.reserves_x - self.treasury_x).untag());
        let y_reserves = BigInt::from((self.reserves_y - self.treasury_y).untag());
        let x_total_fee_pure = BigRational::from_f64((self.lp_fee_x - self.treasury_fee).to_f64()?)?;
        let y_total_fee_pure = BigRational::from_f64((self.lp_fee_x - self.treasury_fee).to_f64()?)?;
        let tr_fee_rev = BigRational::from_f64(1f64 - self.treasury_fee.to_f64().unwrap())?;
        let x_total_fee = x_total_fee_pure.checked_div(&tr_fee_rev)?;
        let y_total_fee = y_total_fee_pure.checked_div(&tr_fee_rev)?;
        let final_spot_price_value = final_spot_price.unwrap();
        let (
            in_balance,
            out_balance,
            w_in,
            w_out,
            fee_mul_num,
            fee_mul_denom,
            corr_fee_num,
            corr_fee_denom,
            final_price_num,
            final_price_denom,
        ) = if (side == Side::Ask && x_is_base) || (side == Side::Bid && !x_is_base) {
            (
                BigInt::from(x_reserves),
                BigInt::from(y_reserves),
                self.weight_x,
                self.weight_y,
                x_total_fee.numer(),
                x_total_fee.denom(),
                x_total_fee_pure.numer(),
                x_total_fee_pure.denom(),
                BigInt::from(*final_spot_price_value.numer()),
                BigInt::from(*final_spot_price_value.denom()),
            )
        } else {
            (
                BigInt::from(y_reserves),
                BigInt::from(x_reserves),
                self.weight_y,
                self.weight_x,
                y_total_fee.numer(),
                y_total_fee.denom(),
                y_total_fee_pure.numer(),
                y_total_fee_pure.denom(),
                BigInt::from(*final_spot_price_value.denom()),
                BigInt::from(*final_spot_price_value.numer()),
            )
        };

        let b_pure = simple_estimate_inp_by_spot_price(
            &in_balance,
            &out_balance,
            &w_in,
            &w_out,
            &final_price_num,
            &final_price_denom,
        )?;
        let b = b_pure.checked_mul(&corr_fee_denom)?.checked_div(&corr_fee_num)?;
        let f_b = spot_price_estimation_error(
            &in_balance,
            &out_balance,
            &b,
            &w_in,
            &w_out,
            &final_price_num,
            &final_price_denom,
            &fee_mul_num,
            &fee_mul_denom,
        )?;

        let mut x_n_1 = b_pure.checked_mul(&corr_fee_num)?.checked_div(&corr_fee_denom)?;
        let max_iter = 50;
        let mut i = 0;
        loop {
            i += 1;
            let f_x_n_1 = spot_price_estimation_error(
                &in_balance,
                &out_balance,
                &x_n_1,
                &w_in,
                &w_out,
                &final_price_num,
                &final_price_denom,
                &fee_mul_num,
                &fee_mul_denom,
            )?;
            let x_n = x_n_1.to_f64().unwrap()
                - f_x_n_1.to_f64().unwrap() * (b.to_f64().unwrap() - x_n_1.to_f64().unwrap())
                    / (f_b.to_f64().unwrap() - f_x_n_1.to_f64().unwrap());
            // let x_n = x_n_1.to_f64().unwrap() - f_x_n_1.to_f64().unwrap() * (b.to_f64().unwrap() - x_n_1.to_f64().unwrap()) / (f_b.to_f64().unwrap() - f_x_n_1.to_f64().unwrap());
            // println!("f_b {}", f_b.to_f64().unwrap());
            // println!("f_x_n_1 {}", f_x_n_1.to_f64().unwrap());
            // println!(
            //     "x_n_pure {}",
            //     x_n
            // );
            // println!(
            //     "x_n {}",
            //     x_n * total_fee_mult_denom.to_f64().unwrap() / total_fee_mult_num.to_f64().unwrap()
            // );
            // println!("b {}", b);
            // println!("Err: {}", (x_n_1.to_f64()? - x_n).abs());
            if (x_n_1.to_f64()? - x_n).abs() < 0.1f64 || i >= max_iter {
                x_n_1 = BigInt::from_f64(x_n)?;
                break;
            }
            x_n_1 = BigInt::from_f64(x_n).unwrap();
        }
        let required_in_amount = x_n_1
            .checked_mul(&tr_fee_rev.denom())?
            .checked_div(&tr_fee_rev.numer())?
            .to_u64()?;
        let input_for_estimation = match side {
            Side::Ask => OnSide::Ask(required_in_amount),
            Side::Bid => OnSide::Bid(required_in_amount),
        };
        self.estimated_trade(input_for_estimation)
    }
    fn full_price_derivative(&self, side: Side, swap_side: SwapAssetSide) -> Option<FullPriceDerivative> {
        let x_asset = self.asset_x.untag();
        let [base, _] = order_canonical(self.asset_x.untag(), self.asset_y.untag());
        let x_is_base = x_asset == base;
        let x_reserves = BigInt::from((self.reserves_x - self.treasury_x).untag());
        let y_reserves = BigInt::from((self.reserves_y - self.treasury_y).untag());
        let w_x_num = BigInt::from(self.weight_x);
        let w_y_num = BigInt::from(self.weight_y);

        let (side_a_balance, side_b_balance, lp_fee_mul_num, lp_fee_mul_denom, w_a_num, w_b_num) =
            if (side == Side::Bid && x_is_base) || (side == Side::Ask && !x_is_base) {
                (
                    x_reserves,
                    y_reserves,
                    BigInt::from(*self.lp_fee_x.numer()),
                    BigInt::from(*self.lp_fee_x.denom()),
                    w_x_num,
                    w_y_num,
                )
            } else {
                (
                    y_reserves,
                    x_reserves,
                    BigInt::from(*self.lp_fee_y.numer()),
                    BigInt::from(*self.lp_fee_y.denom()),
                    w_y_num,
                    w_x_num,
                )
            };
        let out_num = w_b_num
            .checked_mul(&lp_fee_mul_num)?
            .checked_add(&w_a_num.checked_mul(&lp_fee_mul_denom)?)?;
        let out_denom = lp_fee_mul_denom
            .checked_mul(&w_a_num)?
            .checked_mul(&side_b_balance)?;

        let (derivative_num, derivative_denom) = match swap_side {
            SwapAssetSide::Output => (out_num, out_denom),
            SwapAssetSide::Input => (
                w_b_num
                    .checked_mul(&side_a_balance)?
                    .checked_mul(&lp_fee_mul_num)?
                    .checked_mul(&out_num)?,
                out_denom.checked_mul(&out_denom)?,
            ),
        };
        let derivative_big = BigRational::new_raw(derivative_num, derivative_denom);
        let derivative_f64 = derivative_big.to_f64()?;
        Some(FullPriceDerivative(Ratio::from_f64(derivative_f64)?))
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

impl ApplyOrder<ClassicalOnChainDeposit> for BalancePool {
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

impl ApplyOrder<ClassicalOnChainRedeem> for BalancePool {
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
                    .ok_or(ApplyOrderError::incompatible(redeem.clone()))?;

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

#[cfg(test)]
mod tests {
    use std::cmp::{max, min};

    use cml_chain::plutus::PlutusData;
    use cml_chain::Deserialize;
    use cml_core::serialization::Serialize;
    use cml_crypto::{Ed25519KeyHash, ScriptHash, TransactionHash};
    use num_rational::Ratio;
    use num_traits::ToPrimitive;
    use rand::prelude::StdRng;
    use rand::{Rng, SeedableRng};
    use void::Void;

    use algebra_core::semigroup::Semigroup;
    use bloom_offchain::execution_engine::liquidity_book::core::{Next, Trans};
    use bloom_offchain::execution_engine::liquidity_book::market_maker::{
        AvailableLiquidity, MakerBehavior, MarketMaker,
    };
    use bloom_offchain::execution_engine::liquidity_book::side::OnSide;
    use bloom_offchain::execution_engine::liquidity_book::side::OnSide::{Ask, Bid};
    use bloom_offchain::execution_engine::liquidity_book::types::AbsolutePrice;
    use spectrum_cardano_lib::ex_units::ExUnits;
    use spectrum_cardano_lib::types::TryFromPData;
    use spectrum_cardano_lib::{AssetClass, AssetName, OutputRef, TaggedAmount, TaggedAssetClass, Token};

    use crate::data::balance_pool::{BalancePool, BalancePoolConfig, BalancePoolRedeemer, BalancePoolVer};
    use crate::data::order::ClassicalOrder;
    use crate::data::order::OrderType::BalanceFn;
    use crate::data::pool::{ApplyOrder, CFMMPoolAction};
    use crate::data::redeem::{ClassicalOnChainRedeem, Redeem};
    use crate::data::{OnChainOrderId, PoolId};

    const DATUM_SAMPLE: &str = "d8799fd8799f581c5df8fe3f9f0e10855f930e0ea6c227e3bba0aba54d39f9d55b95e21c436e6674ffd8799f4040ff01d8799f581c4b3459fd18a1dbabe207cd19c9951a9fac9f5c0f9c384e3d97efba26457465737443ff04d8799f581c0df79145b95580c14ef4baf8d022d7f0cbb08f3bed43bf97a2ddd8cb426c71ff1a000186820a00009fd8799fd87a9f581cb046b660db0eaf9be4f4300180ccf277e4209dada77c48fbd37ba81dffffff581c8d4be10d934b60a22f267699ea3f7ebdade1f8e535d1bd0ef7ce18b61a0501bced08ff";

    fn gen_ada_token_pool(
        reserves_x: u64,
        reserves_y: u64,
        liquidity: u64,
        lp_fee_x: u64,
        lp_fee_y: u64,
        treasury_fee: u64,
        treasury_x: u64,
        treasury_y: u64,
    ) -> BalancePool {
        return BalancePool {
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
            weight_x: 4,
            reserves_y: TaggedAmount::new(reserves_y),
            weight_y: 1,
            liquidity: TaggedAmount::new(liquidity),
            asset_x: TaggedAssetClass::new(AssetClass::Native),
            asset_y: TaggedAssetClass::new(AssetClass::Token(Token(
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
            ver: BalancePoolVer::V1,
            marginal_cost: ExUnits {
                mem: 120000000,
                steps: 100000000000,
            },
            min_pool_lovelace: 10000000000,
        };
    }

    #[test]
    fn parse_balance_pool_datum() {
        let pd = PlutusData::from_cbor_bytes(&*hex::decode(DATUM_SAMPLE).unwrap()).unwrap();
        let maybe_conf = BalancePoolConfig::try_from_pd(pd);
        assert!(maybe_conf.is_some())
    }

    #[test]
    fn swap() {
        let pool = gen_ada_token_pool(
            2115301811439,
            27887555508598,
            0,
            99000,
            99000,
            100,
            1143236614,
            3057757049,
        );

        let next_pool = pool.swap(OnSide::Ask(200000000));
        let trans = Trans::new(pool, next_pool);

        assert_eq!(trans.loss(), Some(Ask(652178037)))
    }

    #[test]
    fn swap_is_semigroup() {
        let pool = gen_ada_token_pool(
            1145234875102,
            31371708695699,
            9223356007720560247,
            99000,
            99000,
            100,
            2858654886,
            18142984027,
        );

        let input = 449874942;

        let atomic_swap_result = pool.swap(Bid(input));
        let atomic_swap_trans = Trans::new(pool, atomic_swap_result);

        let mut iterative_swap_result: Option<Trans<BalancePool, BalancePool, Void>> = None;
        let mut remaining_input = input;
        let mut pool_in_progress = pool;
        loop {
            if remaining_input > 0 {
                let initial_chunk = input * 25 / 100;
                let chunk = if initial_chunk > 0 {
                    min(initial_chunk, remaining_input)
                } else {
                    remaining_input
                };
                let partial_swap_result = pool_in_progress.swap(Bid(chunk));
                let partial_swap_trans = Trans::new(pool_in_progress, partial_swap_result);
                iterative_swap_result = match iterative_swap_result {
                    None => Some(partial_swap_trans),
                    Some(swap) => Some(swap.combine(partial_swap_trans)),
                };
                let Next::Succ(next_pool) = partial_swap_result else {
                    panic!()
                };
                remaining_input -= chunk;
                pool_in_progress = next_pool;
                continue;
            }
            break;
        }

        assert_eq!(Some(atomic_swap_trans), iterative_swap_result);
    }

    #[test]
    fn swap_redeemer_test() {
        let pool = gen_ada_token_pool(200000000, 84093845, 0, 99970, 99970, 10, 10000, 0);

        let Next::Succ(new_pool) = pool.swap(OnSide::Ask(363613802862)) else {
            panic!()
        };

        let test_swap_redeemer = BalancePoolRedeemer {
            pool_input_index: 0,
            action: CFMMPoolAction::Swap,
            new_pool_state: new_pool,
            prev_pool_state: pool,
        }
        .to_plutus_data();

        assert_eq!(
            hex::encode(test_swap_redeemer.to_canonical_cbor_bytes()),
            "d879850200821b44d28ae9357d3d221b1bfbea3f996900a4821b44d28ae9357d3d221b344bc15514617ce98d18250913185e1318630e0813185d184b185c08"
        )
    }

    #[test]
    fn deposit_redeemer_test() {
        let pool = gen_ada_token_pool(
            1981759952,
            53144,
            9223372036854587823,
            99000,
            99000,
            100,
            13000,
            94,
        );

        const TX: &str = "6c038a69587061acd5611507e68b1fd3a7e7d189367b7853f3bb5079a118b880";
        const IX: u64 = 1;

        let test_order: ClassicalOnChainRedeem = ClassicalOrder {
            id: OnChainOrderId(OutputRef::new(TransactionHash::from_hex(TX).unwrap(), IX)),
            pool_id: pool.id,
            order: Redeem {
                pool_nft: pool.id,
                token_x: pool.asset_x,
                token_y: pool.asset_y,
                token_lq: pool.asset_lq,
                token_lq_amount: TaggedAmount::new(1900727),
                ex_fee: 1500000,
                reward_pkh: Ed25519KeyHash::from([0u8; 28]),
                reward_stake_pkh: None,
                collateral_ada: 3000000,
                order_type: BalanceFn,
            },
        };

        let test = pool.apply_order(test_order);

        let res = test.map(|res| println!("{:?}", res.0));

        assert_eq!(1, 1)
    }

    #[test]
    fn available_liquidity_test() {
        let fee_num = 99000;
        let reserves_x = 1_000_000_000;
        let reserves_y = 2_000_000_000;

        let pool = gen_ada_token_pool(reserves_x, reserves_y, 0, fee_num, fee_num, 0, 0, 0);
        let spot = pool.static_price().unwrap().to_f64().unwrap();
        // ASK:
        let Next::Succ(pool1) = pool.swap(OnSide::Ask(1_000_000)) else {
            panic!()
        };
        let y_rec = pool.reserves_y.untag() - pool1.reserves_y.untag();

        let final_ask_spot = pool1.static_price().unwrap();
        let worst_ask_price = AbsolutePrice::new_raw(
            final_ask_spot.numer() * fee_num as u128,
            final_ask_spot.denom() * *pool.treasury_fee.denom() as u128,
        );

        let Some(AvailableLiquidity {
            input: inp_ask,
            output: out_ask,
        }) = pool.available_liquidity_by_order_price(Ask(worst_ask_price))
        else {
            !panic!();
        };
        // BID:
        let Next::Succ(pool2) = pool.swap(OnSide::Bid(1_000_000)) else {
            panic!()
        };
        let x_rec = pool.reserves_x.untag() - pool2.reserves_x.untag();
        let final_bid_spot = pool2.static_price().unwrap();

        let worst_bid_price = AbsolutePrice::new_raw(
            final_bid_spot.numer() * *pool.treasury_fee.denom() as u128,
            final_bid_spot.denom() * fee_num as u128,
        );

        let Some(AvailableLiquidity {
            input: inp_bid,
            output: out_bid,
        }) = pool.available_liquidity_by_order_price(Bid(worst_bid_price))
        else {
            !panic!();
        };

        assert_eq!(y_rec, 494693);
        assert_eq!(inp_ask, 998878);
        assert_eq!(out_ask, 494444);

        assert_eq!(x_rec, 1977552);
        assert_eq!(inp_bid, 998513);
        assert_eq!(out_bid, 1977057);
    }

    #[test]
    fn available_liquidity_test_new() {
        let fee_num = 86438;
        let reserves_x = 10535678;
        let reserves_y = 217451562;
        let to_swap = 148899250;

        let pool = gen_ada_token_pool(reserves_x, reserves_y, 0, fee_num, fee_num, 4467, 467938, 2060754);
        // ASK:
        let Next::Succ(pool1) = pool.swap(OnSide::Ask(to_swap)) else {
            panic!()
        };
        let y_rec = pool.reserves_y.untag() - pool1.reserves_y.untag();

        let worst_ask_price = AbsolutePrice::new_raw(y_rec as u128, to_swap as u128);
        let Some(AvailableLiquidity {
            input: inp_ask,
            output: out_ask,
        }) = pool.available_liquidity_by_order_price(Ask(worst_ask_price))
        else {
            !panic!();
        };

        // BID:
        let Next::Succ(pool2) = pool.swap(OnSide::Bid(to_swap)) else {
            panic!()
        };
        let x_rec = pool.reserves_x.untag() - pool2.reserves_x.untag();
        let final_bid_spot = pool2.static_price().unwrap();

        let worst_bid_price = AbsolutePrice::new_raw(to_swap as u128, x_rec as u128);

        let Some(AvailableLiquidity {
            input: inp_bid,
            output: out_bid,
        }) = pool.available_liquidity_by_order_price(Bid(worst_bid_price))
        else {
            !panic!();
        };
        println!("diff {}", to_swap as f64 - inp_ask as f64);

        println!("inp_ask {}", inp_ask);
        println!("out_ask {}", out_ask);
        println!("y_rec {}", y_rec);

        println!("inp_bid {}", inp_bid);
        println!("out_bid {}", out_bid);
        println!("x_rec {}", x_rec);

        assert_eq!(1, 2)
    }

    #[test]
    fn available_liquidity_precision_tolerance_test() {
        let mut rng = StdRng::seed_from_u64(42);
        for _ in 0..100 {
            let reserves_x = rng.gen_range(1_000_000..u32::MAX as u64);
            let reserves_y = rng.gen_range(1_000_000..u32::MAX as u64);
            let in_amount = rng.gen_range(100..max(reserves_x, reserves_y));
            let fee_num = rng.gen_range(50000..90000);
            let treasury_fee_num = rng.gen_range(1..9900);
            let treasury_x = rng.gen_range(1..reserves_x / 10);
            let treasury_y = rng.gen_range(1..reserves_y / 10);

            let pool = gen_ada_token_pool(
                reserves_x,
                reserves_y,
                0,
                fee_num,
                fee_num,
                treasury_fee_num,
                treasury_x,
                treasury_y,
            );
            // ASK:
            let Next::Succ(pool1) = pool.swap(OnSide::Ask(in_amount)) else {
                panic!()
            };
            let y_rec = pool.reserves_y.untag() - pool1.reserves_y.untag();

            let worst_ask_price = AbsolutePrice::new_raw(y_rec as u128, in_amount as u128);

            let Some(AvailableLiquidity {
                input: inp_ask,
                output: out_ask,
            }) = pool.available_liquidity_by_order_price(Ask(worst_ask_price))
            else {
                !panic!();
            };
            // BID:
            let Next::Succ(pool2) = pool.swap(OnSide::Bid(in_amount)) else {
                panic!()
            };
            let x_rec = pool.reserves_x.untag() - pool2.reserves_x.untag();

            let worst_bid_price = AbsolutePrice::new_raw(in_amount as u128, x_rec as u128);

            let Some(AvailableLiquidity {
                input: inp_bid,
                output: out_bid,
            }) = pool.available_liquidity_by_order_price(Bid(worst_bid_price))
            else {
                !panic!();
            };

            let base_p = 1000f64;

            println!("in_amount: {}", in_amount);
            println!("inp_ask: {}", inp_ask);
            //
            // assert!((1f64 - in_amount as f64 / inp_ask as f64).abs() * base_p <= 10f64);
            // assert!((1f64 - in_amount as f64 / inp_bid as f64).abs() * base_p <= 10f64);
            println!("est ask: {}", out_ask as f64 / inp_ask as f64);
            println!("real ask: {}", worst_ask_price.unwrap().to_f64().unwrap());
            println!("est bid: {}", out_bid as f64 / inp_bid as f64);
            println!("real bid: {}", worst_bid_price.unwrap().to_f64().unwrap().recip());
            println!("to swap: {}", in_amount);
            println!("rx: {}", reserves_x);
            println!("ry: {}", reserves_y);

            assert!(out_ask as f64 / inp_ask as f64 >= worst_ask_price.unwrap().to_f64().unwrap());
            assert!(out_bid as f64 / inp_bid as f64 >= worst_bid_price.unwrap().to_f64().unwrap().recip());
        }
    }
    #[test]
    fn available_liquidity_by_final_spot_price_test() {
        let lp_fee = 77800;
        let tr_fee = 40000;
        let reserves_x = 2_000_000;
        let reserves_y = 2_000_000;
        let to_swap = 78_392;

        let pool = gen_ada_token_pool(reserves_x, reserves_y, 0, lp_fee, lp_fee, tr_fee, 0, 0);
        // ASK:
        let Next::Succ(pool1) = pool.swap(OnSide::Ask(to_swap)) else {
            panic!()
        };
        let final_spot = pool1.static_price();
        let Some(AvailableLiquidity {
            input: inp_ask,
            output: out_ask,
        }) = pool.available_liquidity_by_spot_price(final_spot.into())
        else {
            !panic!();
        };

        // BID:
        let Next::Succ(pool2) = pool.swap(OnSide::Bid(to_swap)) else {
            panic!()
        };
        let x_rec = pool.reserves_x.untag() - pool2.reserves_x.untag();
        let final_bid_spot = pool2.static_price();

        let worst_bid_price = AbsolutePrice::new_raw(to_swap as u128, x_rec as u128);

        let Some(AvailableLiquidity {
            input: inp_bid,
            output: out_bid,
        }) = pool.available_liquidity_by_spot_price(final_bid_spot.into())
        else {
            !panic!();
        };

        println!("inp_ask {}", inp_ask);
        println!("out_ask {}", out_ask);
        // println!("y_rec {}", y_rec);
        //
        println!("inp_bid {}", inp_bid);
        println!("out_bid {}", out_bid);
        // println!("x_rec {}", x_rec);

        assert_eq!(1, 2)
    }
}
