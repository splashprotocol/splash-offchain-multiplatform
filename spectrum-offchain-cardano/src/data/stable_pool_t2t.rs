use std::fmt::Debug;
use std::ops::Mul;

use bignumber::BigNumber;
use cml_chain::address::Address;
use cml_chain::assets::MultiAsset;
use cml_chain::certs::StakeCredential;
use cml_chain::plutus::{ConstrPlutusData, PlutusData};
use cml_chain::transaction::{ConwayFormatTxOut, DatumOption, TransactionOutput};
use cml_chain::utils::BigInteger;
use cml_chain::Value;
use num_bigint::BigInt;
use num_rational::{BigRational, Ratio};
use num_traits::{
    CheckedAdd, CheckedDiv, CheckedMul, CheckedSub, FromPrimitive, One, Pow, ToPrimitive, Zero,
};
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

use crate::constants::{FEE_DEN, MAX_LQ_CAP};
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
use crate::deployment::ProtocolValidator::StableFnPoolT2T;
use crate::deployment::{DeployedScriptInfo, DeployedValidator, DeployedValidatorErased, RequiresValidator};
use crate::pool_math::cfmm_math::classic_cfmm_shares_amount;
use crate::pool_math::stable_math::stable_cfmm_reward_lp;
use crate::pool_math::stable_pool_t2t_exact_math::{
    calc_stable_swap, calculate_context_values_list, calculate_invariant,
};

const N_TRADABLE_ASSETS: u64 = 2;

#[derive(Debug)]
pub struct StablePoolT2TConfig {
    pub pool_nft: TaggedAssetClass<PoolNft>,
    // max 16k
    pub an2n: u64,
    pub asset_x: TaggedAssetClass<Rx>,
    pub asset_y: TaggedAssetClass<Ry>,
    pub multiplier_x: u64,
    pub multiplier_y: u64,
    pub asset_lq: TaggedAssetClass<Lq>,
    pub lp_fee_num: u64,
    pub treasury_fee_num: u64,
    pub treasury_x: u64,
    pub treasury_y: u64,
}

struct DatumMapping {
    pub pool_nft: usize,
    pub an2n: usize,
    pub asset_x: usize,
    pub asset_y: usize,
    pub multiplier_x: usize,
    pub multiplier_y: usize,
    pub asset_lq: usize,
    pub lp_fee_num: usize,
    pub treasury_fee_num: usize,
    pub treasury_x: usize,
    pub treasury_y: usize,
}

const DATUM_MAPPING: DatumMapping = DatumMapping {
    pool_nft: 0,
    an2n: 1,
    asset_x: 2,
    asset_y: 3,
    multiplier_x: 4,
    multiplier_y: 5,
    asset_lq: 6,
    lp_fee_num: 9,
    treasury_fee_num: 10,
    treasury_x: 13,
    treasury_y: 14,
};

impl TryFromPData for StablePoolT2TConfig {
    fn try_from_pd(data: PlutusData) -> Option<Self> {
        let mut cpd = data.into_constr_pd()?;
        Some(Self {
            pool_nft: TaggedAssetClass::try_from_pd(cpd.take_field(DATUM_MAPPING.pool_nft)?)?,
            an2n: cpd.take_field(DATUM_MAPPING.an2n)?.into_u64()?,
            asset_x: TaggedAssetClass::try_from_pd(cpd.take_field(DATUM_MAPPING.asset_x)?)?,
            asset_y: TaggedAssetClass::try_from_pd(cpd.take_field(DATUM_MAPPING.asset_y)?)?,
            multiplier_x: cpd.take_field(DATUM_MAPPING.multiplier_x)?.into_u64()?,
            multiplier_y: cpd.take_field(DATUM_MAPPING.multiplier_y)?.into_u64()?,
            asset_lq: TaggedAssetClass::try_from_pd(cpd.take_field(DATUM_MAPPING.asset_lq)?)?,
            lp_fee_num: cpd.take_field(DATUM_MAPPING.lp_fee_num)?.into_u64()?,
            treasury_fee_num: cpd.take_field(DATUM_MAPPING.treasury_fee_num)?.into_u64()?,
            treasury_x: cpd.take_field(DATUM_MAPPING.treasury_x)?.into_u64()?,
            treasury_y: cpd.take_field(DATUM_MAPPING.treasury_y)?.into_u64()?,
        })
    }
}

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub enum StablePoolT2TVer {
    V1,
}

impl StablePoolT2TVer {
    pub fn try_from_address<Ctx>(pool_addr: &Address, ctx: &Ctx) -> Option<StablePoolT2TVer>
    where
        Ctx: Has<DeployedScriptInfo<{ StableFnPoolT2T as u8 }>>,
    {
        let maybe_hash = pool_addr.payment_cred().and_then(|c| match c {
            StakeCredential::PubKey { .. } => None,
            StakeCredential::Script { hash, .. } => Some(hash),
        });
        if let Some(this_hash) = maybe_hash {
            if ctx
                .select::<DeployedScriptInfo<{ StableFnPoolT2T as u8 }>>()
                .script_hash
                == *this_hash
            {
                return Some(StablePoolT2TVer::V1);
            }
        };
        None
    }
}

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub struct StablePoolT2T {
    pub id: PoolId,
    pub an2n: u64,
    pub reserves_x: TaggedAmount<Rx>,
    pub multiplier_x: u64,
    pub reserves_y: TaggedAmount<Ry>,
    pub multiplier_y: u64,
    pub liquidity: TaggedAmount<Lq>,
    pub asset_x: TaggedAssetClass<Rx>,
    pub asset_y: TaggedAssetClass<Ry>,
    pub asset_lq: TaggedAssetClass<Lq>,
    pub lp_fee_x: Ratio<u64>,
    pub lp_fee_y: Ratio<u64>,
    pub treasury_fee: Ratio<u64>,
    pub treasury_x: TaggedAmount<Rx>,
    pub treasury_y: TaggedAmount<Ry>,
    pub ver: StablePoolT2TVer,
    /// How many execution units pool invokation costs.
    pub marginal_cost: ExUnits,
}

impl StablePoolT2T {
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
    fn create_redeemer(
        pool_action: CFMMPoolAction,
        pool_in_idx: u64,
        pool_out_idx: u64,
        prev_state: StablePoolT2T,
        new_state: StablePoolT2T,
    ) -> PlutusData {
        let self_ix_pd = PlutusData::Integer(BigInteger::from(pool_in_idx));
        let self_out_pd = PlutusData::Integer(BigInteger::from(pool_out_idx));

        let context_values_list = match pool_action {
            CFMMPoolAction::Swap => {
                let context_values_list =
                    calculate_context_values_list(prev_state, new_state).expect("Invalid pool state");
                ConstrPlutusData::new(
                    0,
                    Vec::from([PlutusData::new_list(vec![PlutusData::new_integer(
                        BigInteger::from(context_values_list.as_u64()),
                    )])]),
                )
            }
            _ => ConstrPlutusData::new(0, Vec::from([PlutusData::new_list(vec![])])),
        };

        PlutusData::ConstrPlutusData(ConstrPlutusData::new(
            0,
            Vec::from([
                self_ix_pd,
                self_out_pd,
                PlutusData::ConstrPlutusData(context_values_list),
            ]),
        ))
    }
}

impl<Ctx> TryFromLedger<TransactionOutput, Ctx> for StablePoolT2T
where
    Ctx: Has<DeployedScriptInfo<{ StableFnPoolT2T as u8 }>> + Has<PoolValidation>,
{
    fn try_from_ledger(repr: &TransactionOutput, ctx: &Ctx) -> Option<Self> {
        if let Some(pool_ver) = StablePoolT2TVer::try_from_address(repr.address(), ctx) {
            let value = repr.value();
            let pd = repr.datum().clone()?.into_pd()?;
            let conf = StablePoolT2TConfig::try_from_pd(pd.clone())?;
            let liquidity_neg = value.amount_of(conf.asset_lq.into())?;
            let bounds = ctx.select::<PoolValidation>();
            let lov = value.amount_of(Native)?;
            let rx = value.amount_of(conf.asset_x.into())?;
            let ry = value.amount_of(conf.asset_y.into())?;
            let reserves_ok = rx > conf.treasury_x && ry > conf.treasury_y;
            let lovelace_ok =
                conf.asset_x.is_native() || conf.asset_y.is_native() || bounds.min_t2t_lovelace <= lov;
            if reserves_ok && lovelace_ok {
                return Some(StablePoolT2T {
                    id: PoolId::try_from(conf.pool_nft).ok()?,
                    an2n: conf.an2n,
                    reserves_x: TaggedAmount::new(value.amount_of(conf.asset_x.into())?),
                    multiplier_x: conf.multiplier_x,
                    reserves_y: TaggedAmount::new(value.amount_of(conf.asset_y.into())?),
                    multiplier_y: conf.multiplier_y,
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
                        .select::<DeployedScriptInfo<{ StableFnPoolT2T as u8 }>>()
                        .marginal_cost,
                });
            }
        }
        None
    }
}

impl IntoLedger<TransactionOutput, ImmutablePoolUtxo> for StablePoolT2T {
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
    cpd.set_field(DATUM_MAPPING.treasury_x, treasury_x.into_pd());
    cpd.set_field(DATUM_MAPPING.treasury_y, treasury_y.into_pd());
}

impl Stable for StablePoolT2T {
    type StableId = PoolId;
    fn stable_id(&self) -> Self::StableId {
        self.id
    }
    fn is_quasi_permanent(&self) -> bool {
        true
    }
}

impl<Ctx> RequiresValidator<Ctx> for StablePoolT2T
where
    Ctx: Has<DeployedValidator<{ StableFnPoolT2T as u8 }>>,
{
    fn get_validator(&self, ctx: &Ctx) -> DeployedValidatorErased {
        match self.ver {
            _ => ctx
                .select::<DeployedValidator<{ StableFnPoolT2T as u8 }>>()
                .erased(),
        }
    }
}

pub struct StablePoolRedeemer {
    pub pool_input_index: u64,
    pub pool_output_index: u64,
    pub action: CFMMPoolAction,
    pub new_pool_state: StablePoolT2T,
    pub prev_pool_state: StablePoolT2T,
}

impl StablePoolRedeemer {
    pub fn to_plutus_data(self) -> PlutusData {
        StablePoolT2T::create_redeemer(
            self.action,
            self.pool_input_index,
            self.pool_output_index,
            self.prev_pool_state,
            self.new_pool_state,
        )
    }
}

impl AMMOps for StablePoolT2T {
    fn output_amount(
        &self,
        base_asset: TaggedAssetClass<Base>,
        base_amount: TaggedAmount<Base>,
    ) -> TaggedAmount<Quote> {
        calc_stable_swap(
            self.asset_x,
            self.reserves_x - self.treasury_x,
            self.multiplier_x,
            self.reserves_y - self.treasury_y,
            self.multiplier_y,
            base_asset,
            base_amount,
            self.an2n,
        )
        .unwrap_or(TaggedAmount::new(0))
    }

    fn reward_lp(
        &self,
        in_x_amount: u64,
        in_y_amount: u64,
    ) -> Option<(TaggedAmount<Lq>, TaggedAmount<Rx>, TaggedAmount<Ry>)> {
        stable_cfmm_reward_lp(
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

impl MakerBehavior for StablePoolT2T {
    fn swap(mut self, input: OnSide<u64>) -> Next<Self, Void> {
        let x = self.asset_x.untag();
        let y = self.asset_y.untag();
        let [base, quote] = order_canonical(x, y);
        let pure_output = match input {
            OnSide::Bid(input) => self
                .output_amount(TaggedAssetClass::new(quote), TaggedAmount::new(input))
                .untag(),
            OnSide::Ask(input) => self
                .output_amount(TaggedAssetClass::new(base), TaggedAmount::new(input))
                .untag(),
        };
        let (base_reserves, lp_fee, quote_reserves) = if x == base {
            (self.reserves_x.as_mut(), self.lp_fee_y, self.reserves_y.as_mut())
        } else {
            (self.reserves_y.as_mut(), self.lp_fee_x, self.reserves_x.as_mut())
        };
        let lp_fees = pure_output * lp_fee.numer() / lp_fee.denom() + 1;

        let mut treasury_fee_ = (pure_output * self.treasury_fee.numer()) / self.treasury_fee.denom();
        let f_rev = lp_fee.denom() - self.treasury_fee.numer() - lp_fee.numer();
        let mut quote_total_delta = pure_output - lp_fees - treasury_fee_ - 1;
        //todo() safe -
        let mut valid_treasury_fee = treasury_fee_ * f_rev >= quote_total_delta * self.treasury_fee.numer();
        let treasury_fee = if valid_treasury_fee || *self.treasury_fee.numer() == 0 {
            treasury_fee_
        } else {
            while !valid_treasury_fee {
                treasury_fee_ += 1;
                quote_total_delta = pure_output - lp_fees - treasury_fee_;
                valid_treasury_fee = treasury_fee_ * f_rev >= quote_total_delta * self.treasury_fee.numer();
            }
            treasury_fee_
        };

        let output = pure_output - treasury_fee - lp_fees;
        match input {
            OnSide::Bid(input) => {
                // A user bid means that they wish to buy the base asset for the quote asset, hence
                // pool reserves of base decreases while reserves of quote increase.
                *quote_reserves += input;
                *base_reserves -= output;
                self.treasury_x = TaggedAmount::new(self.treasury_x.untag() + treasury_fee);
            }
            OnSide::Ask(input) => {
                // User ask is the opposite; sell the base asset for the quote asset.
                *base_reserves += input;
                *quote_reserves -= output;
                self.treasury_y = TaggedAmount::new(self.treasury_y.untag() + treasury_fee);
            }
        }
        Next::Succ(self)
    }
}

impl MarketMaker for StablePoolT2T {
    type U = ExUnits;
    fn static_price(&self) -> SpotPrice {
        let x = self.asset_x.untag();
        let y = self.asset_y.untag();

        let x_calc = (self.reserves_x.untag() - self.treasury_x.untag()) * self.multiplier_x;
        let y_calc = (self.reserves_y.untag() - self.treasury_y.untag()) * self.multiplier_y;

        let an2n = BigInt::from(self.an2n);
        let in_calc = BigInt::from(x_calc);
        let out_calc = BigInt::from(y_calc);
        let d = BigInt::from(
            calculate_invariant(
                &U512::from(in_calc.to_u64().unwrap()),
                &U512::from(out_calc.to_u64().unwrap()),
                &U512::from(an2n.to_u64().unwrap()),
            )
            .unwrap()
            .as_u128(),
        );
        let alpha = vec![d; (N_TRADABLE_ASSETS + 1) as usize]
            .into_iter()
            .fold(BigInt::one(), |a, b| a * b);

        let c = an2n
            .checked_mul(&in_calc)
            .unwrap()
            .checked_mul(&in_calc)
            .unwrap()
            .checked_mul(&out_calc)
            .unwrap()
            .checked_mul(&out_calc)
            .unwrap();

        let big_price_num = alpha.checked_mul(&in_calc).unwrap().checked_add(&c).unwrap();
        let big_price_denom = alpha.checked_mul(&out_calc).unwrap().checked_add(&c).unwrap();
        let big_price = BigRational::new_raw(big_price_num.into(), big_price_denom.into());
        let u64_price = Ratio::<u64>::from_f64(big_price.to_f64().unwrap()).unwrap();

        let [base, _] = order_canonical(x, y);
        if x == base {
            AbsolutePrice::new_unsafe(*u64_price.numer(), *u64_price.denom()).into()
        } else {
            AbsolutePrice::new_unsafe(*u64_price.denom(), *u64_price.numer()).into()
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

    fn available_liquidity_by_order_price(
        &self,
        worst_price: OnSide<AbsolutePrice>,
    ) -> Option<AvailableLiquidity> {
        // let x_asset = self.asset_x.untag();
        // let [base, _] = order_canonical(self.asset_x.untag(), self.asset_y.untag());
        // let x_is_base = x_asset == base;
        //
        // let x_reserves = BigInt::from((self.reserves_x - self.treasury_x).untag());
        // let y_reserves = BigInt::from((self.reserves_y - self.treasury_y).untag());
        // let mul_x = BigInt::from(self.multiplier_x);
        // let mul_y = BigInt::from(self.multiplier_y);
        //
        // let worst_price_value = worst_price.unwrap();
        // let price_num = BigInt::from(*worst_price_value.numer());
        // let price_denom = BigInt::from(*worst_price_value.denom());
        //
        // // Side of the operation:
        // let (side,) = match worst_price {
        //     OnSide::Ask(_) => (Side::Ask,),
        //     OnSide::Bid(_) => (Side::Bid,),
        // };
        //
        // let spot_price_orig_f64 = self.static_price().unwrap().to_f64()?;
        // let (
        //     in_balance,
        //     out_balance,
        //     total_fee_mult_num,
        //     total_fee_mult_denom,
        //     final_spot_price_num,
        //     final_spot_price_denom,
        //     spot_price_f64,
        //     mul_in,
        //     mul_out,
        // ) = if (side == Side::Ask && x_is_base) || (side == Side::Bid && !x_is_base) {
        //     (
        //         x_reserves,
        //         y_reserves,
        //         BigInt::from(*self.lp_fee_y.denom() - *self.lp_fee_y.numer() - *self.treasury_fee.numer()),
        //         BigInt::from(*self.lp_fee_y.denom()),
        //         price_num,
        //         price_denom,
        //         spot_price_orig_f64,
        //         mul_x,
        //         mul_y,
        //     )
        // } else {
        //     (
        //         y_reserves,
        //         x_reserves,
        //         BigInt::from(*self.lp_fee_x.denom() - *self.lp_fee_x.numer() - *self.treasury_fee.numer()),
        //         BigInt::from(*self.lp_fee_x.denom()),
        //         price_denom,
        //         price_num,
        //         spot_price_orig_f64.recip(),
        //         mul_y,
        //         mul_x,
        //     )
        // };
        // let derivative_in = self.full_price_derivative(side, SwapAssetSide::Input)?.0;
        // let derivative_in_num = BigInt::try_from(*derivative_in.numer()).ok()?;
        // let derivative_in_denom = BigInt::try_from(*derivative_in.denom()).ok()?;
        //
        // let avg_price_num = final_spot_price_num.checked_mul(&total_fee_mult_denom)?;
        // let avg_price_denom = final_spot_price_denom.checked_mul(&total_fee_mult_num)?;
        // let avg_price_f64 = BigRational::new(avg_price_num.clone(), avg_price_denom.clone()).to_f64()?;
        // println!("avg_price_f64: {}", avg_price_f64);
        // let scale_factor = 2f64;
        // let target_price_delta_f64 = scale_factor * (spot_price_f64 - avg_price_f64);
        // println!("target_price_delta_f64: {}", target_price_delta_f64);
        // let target_price_delta = BigRational::from_f64(target_price_delta_f64)?;
        // let est_in = total_fee_mult_num
        //     .checked_mul(&target_price_delta.numer())?
        //     .checked_mul(&derivative_in_denom)?
        //     .checked_div(
        //         &total_fee_mult_denom
        //             .checked_mul(&target_price_delta.denom())?
        //             .checked_mul(&derivative_in_num)?,
        //     )?;
        let (side,) = match worst_price {
            OnSide::Ask(_) => (Side::Ask,),
            OnSide::Bid(_) => (Side::Bid,),
        };
        let [base, _] = order_canonical(self.asset_x.untag(), self.asset_y.untag());

        let tradable_x_reserves = (self.reserves_x - self.treasury_x).untag();
        let tradable_y_reserves = (self.reserves_y - self.treasury_y).untag();
        let fee_x = BigNumber::from((self.lp_fee_x - self.treasury_fee).to_f64()?);
        let fee_y = BigNumber::from((self.lp_fee_y - self.treasury_fee).to_f64()?);
        let bid_price = BigNumber::from(*worst_price.unwrap().denom() as f64)
            / BigNumber::from(*worst_price.unwrap().numer() as f64);
        let ask_price = BigNumber::from(*worst_price.unwrap().numer() as f64)
            / BigNumber::from(*worst_price.unwrap().denom() as f64);

        let (
            tradable_reserves_base,
            tradable_reserves_quote,
            multiplier_base,
            multiplier_quote,
            total_fee_mult,
            avg_price,
        ) = match worst_price {
            OnSide::Bid(_) if base == self.asset_x.untag() => (
                tradable_y_reserves,
                tradable_x_reserves,
                self.multiplier_y,
                self.multiplier_x,
                fee_x,
                bid_price,
            ),
            OnSide::Bid(_) => (
                tradable_x_reserves,
                tradable_y_reserves,
                self.multiplier_x,
                self.multiplier_y,
                fee_y,
                bid_price,
            ),
            OnSide::Ask(_) if base == self.asset_x.untag() => (
                tradable_x_reserves,
                tradable_y_reserves,
                self.multiplier_x,
                self.multiplier_y,
                fee_y,
                ask_price,
            ),
            OnSide::Ask(_) => (
                tradable_y_reserves,
                tradable_x_reserves,
                self.multiplier_y,
                self.multiplier_x,
                fee_x,
                ask_price,
            ),
        };

        const N2N: f32 = 16.0;
        let sqrt_degree = BigNumber::from(0.5);
        let n_1 = BigNumber::from(1);
        let n_2 = BigNumber::from(2);
        let n_4 = BigNumber::from(4);
        let n_8 = BigNumber::from(8);
        let n_16 = BigNumber::from(16);
        let n_32 = BigNumber::from(32);
        let n_48 = BigNumber::from(48);
        let n_64 = BigNumber::from(64);

        let x0_orig = tradable_reserves_base * multiplier_base;
        let y0_orig = tradable_reserves_quote * multiplier_quote;
        let x0 = BigNumber::from(x0_orig as f64);
        let y0 = BigNumber::from(y0_orig as f64);
        let ampl_coeff = BigNumber::from(self.an2n as f64) / BigNumber::from(N2N);
        let ampl_coeff2 = ampl_coeff.clone() * ampl_coeff.clone();
        let d = BigNumber::from(calculate_invariant(
            &U512::from(x0_orig),
            &U512::from(y0_orig),
            &U512::from(self.an2n),
        )?);

        let d2 = d.clone() * d.clone();
        let ampl_coeff_d = ampl_coeff.clone() * d.clone();
        let ampl_coeff2_d2 = ampl_coeff2.clone() * d2.clone();

        let x_init = x0.clone();
        let mut x = n_2.clone() * x_init.clone();
        let mut k = n_1.clone();

        let mut counter = 0;
        let mut err = x0_orig as i64;
        while err.abs() > 1 && counter < 255 {
            let x2 = x.clone() * x.clone();
            let x3 = x2.clone() * x.clone();
            let d2_x = d2.clone() * x.clone();
            let ampl_coeff2_d2_x = ampl_coeff2.clone() * d2_x.clone();
            let c = n_16.clone() * ampl_coeff2_d2_x.clone()
                - n_32.clone() * ampl_coeff2.clone() * d.clone() * x2.clone()
                + n_16.clone() * ampl_coeff2.clone() * x3.clone()
                + n_4.clone() * ampl_coeff_d.clone() * d2.clone()
                - n_8.clone() * ampl_coeff.clone() * d2.clone() * x.clone()
                + n_8.clone() * ampl_coeff_d.clone() * x2.clone()
                + d2.clone() * x.clone();
            k = n_4.clone() * (x.clone() * c.clone()).pow(&sqrt_degree);
            let l =
                d.clone() - n_4.clone() * ampl_coeff_d.clone() + n_4.clone() * ampl_coeff.clone() * x.clone();
            let twice_cx = n_2.clone() * c.clone() * x.clone();
            let add_num = twice_cx.clone()
                * (x0.clone() - x.clone())
                * (n_32.clone()
                    * ampl_coeff.clone()
                    * avg_price.clone()
                    * x.clone()
                    * (x.clone() - x0.clone())
                    + (total_fee_mult.clone() - n_1.clone())
                        * (n_32.clone() * ampl_coeff.clone() * x.clone() * y0.clone() - k.clone()
                            + n_4.clone() * l.clone() * x.clone()));
            let add_denom = (total_fee_mult.clone() - n_1.clone())
                * (twice_cx.clone()
                    * (n_32.clone() * ampl_coeff.clone() * x.clone() * y0.clone() - k.clone()
                        + n_4.clone() * l.clone() * x.clone())
                    + (x.clone() - x0.clone())
                        * (k.clone()
                            * (n_16.clone() * ampl_coeff2_d2_x.clone()
                                - n_32.clone() * ampl_coeff2.clone() * d.clone() * x2.clone()
                                + n_16.clone() * ampl_coeff2.clone() * x3.clone()
                                + n_4.clone() * ampl_coeff_d.clone() * d2.clone()
                                - n_8.clone() * ampl_coeff.clone() * d2_x.clone()
                                + n_8.clone() * ampl_coeff_d.clone() * x2.clone()
                                + d2_x.clone()
                                + x.clone()
                                    * (n_16.clone() * ampl_coeff2_d2.clone()
                                        - n_64.clone() * ampl_coeff2.clone() * d.clone() * x.clone()
                                        + n_48.clone() * ampl_coeff2.clone() * x2.clone()
                                        - n_8.clone() * ampl_coeff.clone() * d2.clone()
                                        + n_16.clone() * ampl_coeff_d.clone() * x.clone()
                                        + d2.clone()))
                            - n_8.clone()
                                * c.clone()
                                * x.clone()
                                * (n_8.clone() * ampl_coeff.clone() * x.clone()
                                    - n_4.clone() * ampl_coeff_d.clone()
                                    + d.clone())
                            + c.clone() * (n_8.clone() * l.clone() * x.clone() - n_2.clone() * k.clone())));
            let add = add_num / add_denom;

            err = <i64>::try_from(add.value.to_int().value()).ok()?;
            let x_new = <i64>::try_from(x.value.to_int().value()).ok()? - err;
            if x_new > 0 {
                x = BigNumber::from(x_new as f64);
            } else {
                break;
            }
            counter += 1
        }

        let base_delta = x.clone() - x0.clone();
        let input_amount_val = <u64>::try_from(base_delta.value.to_int().value()).ok()?;

        let input_for_estimation = match side {
            Side::Ask => OnSide::Ask(input_amount_val),
            Side::Bid => OnSide::Bid(input_amount_val),
        };
        self.estimated_trade(input_for_estimation)
    }

    fn full_price_derivative(&self, side: Side, swap_side: SwapAssetSide) -> Option<FullPriceDerivative> {
        let x_asset = self.asset_x.untag();
        let [base, _] = order_canonical(self.asset_x.untag(), self.asset_y.untag());
        let x_is_base = x_asset == base;
        let x_reserves = BigInt::from((self.reserves_x - self.treasury_x).untag());
        let y_reserves = BigInt::from((self.reserves_y - self.treasury_y).untag());
        let mul_x = BigInt::from(self.multiplier_x);
        let mul_y = BigInt::from(self.multiplier_y);

        let (side_a_balance, side_b_balance, lp_fee_mul_num, lp_fee_mul_denom, side_a_mul, side_b_mul) =
            if (side == Side::Bid && x_is_base) || (side == Side::Ask && !x_is_base) {
                (
                    x_reserves,
                    y_reserves,
                    BigInt::from(*self.lp_fee_y.denom() - *self.lp_fee_y.numer()),
                    BigInt::from(*self.lp_fee_y.denom()),
                    mul_x,
                    mul_y,
                )
            } else {
                (
                    y_reserves,
                    x_reserves,
                    BigInt::from(*self.lp_fee_x.denom() - *self.lp_fee_x.numer()),
                    BigInt::from(*self.lp_fee_x.denom()),
                    mul_y,
                    mul_x,
                )
            };
        let x_calc = side_a_balance.checked_mul(&side_a_mul)?;
        let y_calc = side_b_balance.checked_mul(&side_b_mul)?;
        let n_2 = BigInt::from(2);
        let x2 = x_calc.checked_mul(&x_calc)?;
        let x2y_2 = n_2.checked_mul(&x2)?.checked_mul(&y_calc)?;
        let y2 = y_calc.checked_mul(&y_calc)?;
        let y2x_2 = n_2.checked_mul(&y2)?.checked_mul(&x_calc)?;
        let b = x2.checked_mul(&y2)?;
        let an2n = BigInt::from(self.an2n);
        let d = BigInt::from(
            calculate_invariant(
                &U512::from(x_calc.to_u64().unwrap()),
                &U512::from(y_calc.to_u64().unwrap()),
                &U512::from(an2n.to_u64().unwrap()),
            )?
            .as_u128(),
        );
        let dn1 = vec![d; (N_TRADABLE_ASSETS + 1) as usize]
            .into_iter()
            .fold(BigInt::one(), |a, b| a * b);
        let alpha = dn1.checked_div(&an2n)?;
        let c = alpha.checked_mul(&x_calc)?.checked_add(&b)?;
        let d = alpha.checked_mul(&y_calc)?.checked_add(&b)?;
        let cd = c.checked_mul(&d)?;
        let omega = cd
            .checked_mul(&alpha.checked_add(&x2y_2)?)?
            .checked_sub(&x2y_2.checked_mul(&d)?.checked_mul(&d)?)?
            .checked_mul(&lp_fee_mul_denom)?;

        let gamma = cd
            .checked_mul(&alpha.checked_add(&y2x_2)?)?
            .checked_sub(&y2x_2.checked_mul(&c)?.checked_mul(&c)?)?
            .checked_mul(&lp_fee_mul_num)?;

        let out_num = omega.checked_add(&gamma)?;
        let out_denom = cd.checked_mul(&lp_fee_mul_denom)?.checked_mul(&d)?;

        let (derivative_num, derivative_denom) = match swap_side {
            SwapAssetSide::Output => (out_num, out_denom),
            SwapAssetSide::Input => (
                lp_fee_mul_num.checked_mul(&omega.checked_mul(&d)?.checked_add(&gamma.checked_mul(&c)?)?)?,
                out_denom.checked_mul(&lp_fee_mul_denom)?.checked_mul(&d)?,
            ),
        };

        let big_der = BigRational::new_raw(derivative_num.into(), derivative_denom.into());
        let u128_der = Ratio::<u128>::from_f64(big_der.to_f64()?)?;

        Some(FullPriceDerivative(Ratio::new_raw(
            *u128_der.numer(),
            *u128_der.denom(),
        )))
    }

    fn estimated_trade(&self, input: OnSide<u64>) -> Option<AvailableLiquidity> {
        let x = self.asset_x.untag();
        let y = self.asset_y.untag();
        let [base, quote] = order_canonical(x, y);
        let x_is_base = x == base;
        let (pure_output, side) = match input {
            OnSide::Bid(input) => (
                self.output_amount(TaggedAssetClass::new(quote), TaggedAmount::new(input))
                    .untag(),
                Side::Bid,
            ),
            OnSide::Ask(input) => (
                self.output_amount(TaggedAssetClass::new(base), TaggedAmount::new(input))
                    .untag(),
                Side::Ask,
            ),
        };
        let (fee_num, fee_denom) = if (side == Side::Ask && x_is_base) || (side == Side::Bid && !x_is_base) {
            (
                BigInt::from(*self.lp_fee_y.denom() - *self.lp_fee_y.numer() - *self.treasury_fee.numer()),
                BigInt::from(*self.lp_fee_y.denom()),
            )
        } else {
            (
                BigInt::from(*self.lp_fee_x.denom() - *self.lp_fee_x.numer() - *self.treasury_fee.numer()),
                BigInt::from(*self.lp_fee_x.denom()),
            )
        };

        let pure_output_big = BigInt::from(pure_output);

        Some(AvailableLiquidity {
            input: input.unwrap(),
            output: pure_output_big
                .checked_mul(&fee_num)?
                .checked_div(&fee_denom)?
                .to_u64()?,
        })
    }

    fn is_active(&self) -> bool {
        // stable pools doesn't support lq bound, so
        // swaps are allowed all time
        true
    }

    fn available_liquidity_by_spot_price(&self, final_spot_price: SpotPrice) -> Option<AvailableLiquidity> {
        let x_asset = self.asset_x.untag();
        let [base, _] = order_canonical(self.asset_x.untag(), self.asset_y.untag());
        let x_is_base = x_asset == base;

        let mul_x = BigInt::from(self.multiplier_x);
        let mul_y = BigInt::from(self.multiplier_y);

        let final_spot_price_value = final_spot_price.unwrap();
        let price_num = BigInt::from(*final_spot_price_value.numer());
        let price_denom = BigInt::from(*final_spot_price_value.denom());

        let side = if x_is_base && final_spot_price_value.to_f64() > self.static_price().unwrap().to_f64() {
            Side::Ask
        } else {
            Side::Bid
        };
        println!("side: {}", side);

        let spot_price_orig_f64 = self.static_price().unwrap().to_f64()?;
        let (fee_denom, lp_fee_mult_num, mul_in) =
            if (side == Side::Ask && x_is_base) || (side == Side::Bid && !x_is_base) {
                (
                    BigInt::from(*self.lp_fee_y.denom()),
                    BigInt::from(*self.lp_fee_y.denom() - *self.lp_fee_y.numer()),
                    mul_x,
                )
            } else {
                (
                    BigInt::from(*self.lp_fee_x.denom()),
                    BigInt::from(*self.lp_fee_x.denom() - *self.lp_fee_x.numer()),
                    mul_y,
                )
            };

        let final_price_f64 = BigRational::new(price_num.clone(), price_denom.clone()).to_f64()?;
        let target_price_delta_f64 = spot_price_orig_f64 - final_price_f64;
        let target_price_delta = BigRational::from_f64(target_price_delta_f64)?;
        let min_step = 1e-7;
        let min_step_rational = BigRational::from_f64(min_step)?;
        let n_steps = (target_price_delta_f64 / min_step).to_i64()?.abs() + 1;
        println!("n_steps: {}", target_price_delta_f64 / min_step);
        println!("n_steps: {}", n_steps);
        println!("target_price_delta: {}", target_price_delta_f64);
        println!("spot_price_orig_f64: {}", spot_price_orig_f64);

        let est_in_value = {
            if n_steps == 1 {
                let derivative_in = self.full_price_derivative(side, SwapAssetSide::Input)?.0;
                let derivative_in_num = BigInt::try_from(*derivative_in.numer()).ok()?;
                let derivative_in_denom = BigInt::try_from(*derivative_in.denom()).ok()?;
                target_price_delta
                    .numer()
                    .checked_mul(&derivative_in_denom)?
                    .checked_mul(&lp_fee_mult_num)?
                    .checked_div(
                        &target_price_delta
                            .denom()
                            .checked_mul(&derivative_in_num)?
                            .checked_mul(&fee_denom)?
                            .checked_mul(&mul_in)?,
                    )?
            } else {
                let mut pool = self.clone();
                let mut est_in_acc = BigInt::zero();
                let mut acc_delta = BigRational::zero();
                for i in 0..2 * n_steps {
                    println!("i: {}", i);
                    let derivative_in = pool.full_price_derivative(side, SwapAssetSide::Input)?.0;
                    let derivative_in_num = BigInt::try_from(*derivative_in.numer()).ok()?;
                    let derivative_in_denom = BigInt::try_from(*derivative_in.denom()).ok()?;
                    let step = if i < n_steps - 1 {
                        min_step_rational.clone()
                    } else {
                        let d_n = BigRational::from_u64(100)?;
                        min_step_rational.clone().checked_div(&d_n)?
                    };
                    acc_delta = acc_delta + step.clone();
                    println!("step: {}", step.to_f64().unwrap());

                    let est_in_add = step
                        .numer()
                        .checked_mul(&derivative_in_denom)?
                        .checked_mul(&lp_fee_mult_num)?
                        .checked_div(
                            &step
                                .denom()
                                .checked_mul(&derivative_in_num)?
                                .checked_mul(&fee_denom)?
                                .checked_mul(&mul_in)?,
                        )?;
                    let iter_inp = match side {
                        Side::Ask => OnSide::Ask(est_in_add.to_u64()?),
                        Side::Bid => OnSide::Bid(est_in_add.to_u64()?),
                    };
                    let Next::Succ(pool_succ) = pool.swap(iter_inp) else {
                        panic!()
                    };
                    println!("est_in_add: {}", est_in_add.to_i64().unwrap());
                    pool = pool_succ;
                    println!(
                        "pool.static_price().unwrap().to_f64()?: {}",
                        pool.static_price().unwrap().to_f64()?
                    );
                    if side == Side::Ask {
                        if pool.static_price().unwrap().to_f64()? > final_price_f64 {
                            break;
                        }
                    } else {
                        if pool.static_price().unwrap().to_f64()? < final_price_f64 {
                            break;
                        }
                    }

                    est_in_acc = est_in_acc.checked_add(&est_in_add)?;
                }
                println!("acc_delta: {}", acc_delta.to_f64().unwrap());
                est_in_acc
            }
        };
        println!("est_in_value: {}", est_in_value.to_u64()?);
        let input_for_estimation = match side {
            Side::Ask => OnSide::Ask(est_in_value.to_u64()?),
            Side::Bid => OnSide::Bid(est_in_value.to_u64()?),
        };
        self.estimated_trade(input_for_estimation)
    }
}

impl ApplyOrder<ClassicalOnChainDeposit> for StablePoolT2T {
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

impl ApplyOrder<ClassicalOnChainRedeem> for StablePoolT2T {
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
    use cml_chain::plutus::PlutusData;
    use cml_chain::Deserialize;
    use cml_crypto::{Ed25519KeyHash, ScriptHash, TransactionHash};
    use num_rational::Ratio;
    use num_traits::ToPrimitive;
    use primitive_types::U512;
    use rand::prelude::StdRng;
    use rand::{Rng, SeedableRng};

    use bloom_offchain::execution_engine::liquidity_book::core::Next;
    use bloom_offchain::execution_engine::liquidity_book::market_maker::{
        AvailableLiquidity, FullPriceDerivative, MakerBehavior, MarketMaker,
    };
    use bloom_offchain::execution_engine::liquidity_book::side::OnSide::{Ask, Bid};
    use bloom_offchain::execution_engine::liquidity_book::side::{OnSide, Side, SwapAssetSide};
    use bloom_offchain::execution_engine::liquidity_book::types::AbsolutePrice;
    use spectrum_cardano_lib::ex_units::ExUnits;
    use spectrum_cardano_lib::types::TryFromPData;
    use spectrum_cardano_lib::{AssetClass, AssetName, OutputRef, TaggedAmount, TaggedAssetClass, Token};

    use crate::constants::MAX_LQ_CAP;
    use crate::data::order::ClassicalOrder;
    use crate::data::order::OrderType::BalanceFn;
    use crate::data::pool::ApplyOrder;
    use crate::data::redeem::{ClassicalOnChainRedeem, Redeem};
    use crate::data::stable_pool_t2t::{StablePoolT2T, StablePoolT2TConfig, StablePoolT2TVer};
    use crate::data::{OnChainOrderId, PoolId};
    use crate::pool_math::stable_pool_t2t_exact_math::{
        calculate_invariant, calculate_safe_price_ratio_x_y_swap,
    };

    const DATUM_SAMPLE: &str = "d8799fd8799f581c7dbe6f0c7849e2dae806cd4681910bfe1bbc0d5fd4e370e8e2f7bd4a436e6674ff190c80d8799f4040ffd8799f581c4b3459fd18a1dbabe207cd19c9951a9fac9f5c0f9c384e3d97efba26457465737443ff0000d8799f581c6abe65f6adc8301ff4dbfcfcec1a187075639d21f85cae3c1cf2a060426c71ffd87980d879801a000186820a581c4b3459fd18a1dbabe207cd19c9951a9fac9f5c0f9c384e3d97efba26581c4b3459fd18a1dbabe207cd19c9951a9fac9f5c0f9c384e3d97efba260000ff";

    fn gen_ada_token_pool(
        reserves_x: u64,
        x_decimals: u32,
        reserves_y: u64,
        y_decimals: u32,
        lp_fee_x: u64,
        lp_fee_y: u64,
        treasury_fee: u64,
        treasury_x: u64,
        treasury_y: u64,
        an2n: u64,
    ) -> StablePoolT2T {
        let reserves_x = reserves_x;
        let reserves_y = reserves_y;
        let (multiplier_x, multiplier_y) = if (x_decimals > y_decimals) {
            (1, 10_u32.pow(x_decimals - y_decimals))
        } else if (x_decimals < y_decimals) {
            (10_u32.pow(y_decimals - x_decimals), 1)
        } else {
            (1, 1)
        };
        let inv_before = calculate_invariant(
            &U512::from((reserves_x - treasury_x) * multiplier_x as u64),
            &U512::from((reserves_y - treasury_y) * multiplier_y as u64),
            &U512::from(an2n),
        )
        .unwrap();
        let liquidity = MAX_LQ_CAP - inv_before.as_u64();

        return StablePoolT2T {
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
            an2n: an2n, // constant
            reserves_x: TaggedAmount::new(reserves_x),
            multiplier_x: multiplier_x as u64,
            reserves_y: TaggedAmount::new(reserves_y),
            multiplier_y: multiplier_y as u64,
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
            ver: StablePoolT2TVer::V1,
            marginal_cost: ExUnits {
                mem: 120000000,
                steps: 100000000000,
            },
        };
    }

    #[test]
    fn parse_stable_pool_t2t_datum() {
        let pd = PlutusData::from_cbor_bytes(&*hex::decode(DATUM_SAMPLE).unwrap()).unwrap();
        let maybe_conf = StablePoolT2TConfig::try_from_pd(pd);
        assert!(maybe_conf.is_some())
    }

    #[test]
    fn swap() {
        // Swap to min decimals;
        let pool = gen_ada_token_pool(
            475000220,
            6,
            343088,
            3,
            20000,
            20000,
            50000,
            220000220,
            88088,
            300 * 16,
        );

        let Next::Succ(result) = pool.swap(OnSide::Bid(390088 - 343088)) else {
            panic!()
        };

        assert_eq!(result.reserves_x.untag(), 460904695);
        assert_eq!(result.reserves_y.untag(), 390088);
        assert_eq!(result.treasury_x.untag(), 243492762);
        assert_eq!(result.treasury_y.untag(), 88088);

        // Swap to max decimals;
        let pool = gen_ada_token_pool(
            343088,
            3,
            475000220,
            6,
            20000,
            20000,
            50000,
            88088,
            220000220,
            300 * 16,
        );

        let Next::Succ(result) = pool.swap(OnSide::Ask(390088 - 343088)) else {
            panic!()
        };

        assert_eq!(result.reserves_x.untag(), 390088);
        assert_eq!(result.reserves_y.untag(), 460904695);
        assert_eq!(result.treasury_x.untag(), 88088);
        assert_eq!(result.treasury_y.untag(), 243492762);

        // Uniform swap;
        let pool = gen_ada_token_pool(100000, 1, 100000, 1, 2000, 2000, 5000, 0, 0, 300 * 16);

        let Next::Succ(result) = pool.swap(OnSide::Ask(1000)) else {
            panic!()
        };

        assert_eq!(result.reserves_x.untag(), 101000);
        assert_eq!(result.reserves_y.untag(), 99071);
        assert_eq!(result.treasury_x.untag(), 0);
        assert_eq!(result.treasury_y.untag(), 50);

        // Some swap;
        let pool = gen_ada_token_pool(100100000, 1, 99900201, 1, 100, 100, 100, 0, 100, 300 * 16);

        let Next::Succ(result) = pool.swap(OnSide::Ask(100000)) else {
            panic!()
        };

        assert_eq!(result.reserves_x.untag(), 100200000);
        assert_eq!(result.reserves_y.untag(), 99800403);
        assert_eq!(result.treasury_x.untag(), 0);
        assert_eq!(result.treasury_y.untag(), 200);

        // Uniform swap;
        let pool = gen_ada_token_pool(108500000, 1, 108500000, 1, 100, 100, 100, 0, 0, 200 * 16);

        let Next::Succ(result) = pool.swap(OnSide::Ask(100000000)) else {
            panic!()
        };

        assert_eq!(result.reserves_x.untag(), 208500000);
        assert_eq!(result.reserves_y.untag(), 9990110);
        assert_eq!(result.treasury_x.untag(), 0);
        assert_eq!(result.treasury_y.untag(), 98708);
    }

    #[test]
    fn deposit_redeemer_test() {
        let pool = gen_ada_token_pool(
            1_000_000_000,
            9,
            1_000_000_000,
            9,
            99000,
            99000,
            100,
            0,
            0,
            300 * 16,
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

        assert_eq!(1, 1)
    }

    #[test]
    fn full_price_derivative_test() {
        let pool = gen_ada_token_pool(1000000000, 1, 1200000000, 1, 4500, 4500, 0, 0, 0, 200 * 16);

        // Input:
        let Some(FullPriceDerivative(ask_in_d)) = pool.full_price_derivative(Side::Ask, SwapAssetSide::Input)
        else {
            panic!()
        };
        let Some(FullPriceDerivative(bid_in_d)) = pool.full_price_derivative(Side::Bid, SwapAssetSide::Input)
        else {
            panic!()
        };

        // Output:
        let Some(FullPriceDerivative(ask_out_d)) =
            pool.full_price_derivative(Side::Ask, SwapAssetSide::Output)
        else {
            panic!()
        };
        let Some(FullPriceDerivative(bid_out_d)) =
            pool.full_price_derivative(Side::Bid, SwapAssetSide::Output)
        else {
            panic!()
        };

        assert_eq!(ask_out_d.to_f64().unwrap(), 4.694401760666852e-12);
        assert_eq!(bid_out_d.to_f64().unwrap(), 4.616390692169021e-12);
        assert_eq!(ask_in_d.to_f64().unwrap(), 4.4838007124958665e-12);
        assert_eq!(bid_in_d.to_f64().unwrap(), 4.407298232114253e-12);
    }

    #[test]
    fn available_liquidity_by_order_price_test() {
        let lp_fee_num = 13200;
        let tr_fee_num = 30000;
        let reserves_x = 1_000_000_000_000;
        let reserves_y = 1_000_000_000_000;

        let pool = gen_ada_token_pool(
            reserves_x,
            0,
            reserves_y,
            0,
            lp_fee_num,
            lp_fee_num,
            tr_fee_num,
            0,
            0,
            200 * 16,
        );
        let spot = pool.static_price().unwrap().to_f64().unwrap();
        let base_in = 1_000_000;
        // ASK:
        let Next::Succ(pool1) = pool.swap(OnSide::Ask(base_in)) else {
            panic!()
        };
        let y_rec = pool.reserves_y.untag() - pool1.reserves_y.untag();

        let final_ask_spot = pool1.static_price().unwrap();
        println!("final_ask_spot: {}", final_ask_spot.to_f64().unwrap().recip());
        let fee_num_rev = pool.treasury_fee.denom() - tr_fee_num;

        let worst_ask_price = AbsolutePrice::new_raw(y_rec as u128, base_in as u128);

        let Some(AvailableLiquidity {
            input: inp_ask,
            output: out_ask,
        }) = pool.available_liquidity_by_order_price(Ask(worst_ask_price))
        else {
            !panic!();
        };
        // BID:
        let Next::Succ(pool2) = pool.swap(OnSide::Bid(base_in)) else {
            panic!()
        };
        let x_rec = pool.reserves_x.untag() - pool2.reserves_x.untag();
        let final_bid_spot = pool2.static_price().unwrap();
        println!("final_bid_spot: {}", final_bid_spot.to_f64().unwrap());

        let worst_bid_price = AbsolutePrice::new_raw(base_in as u128, y_rec as u128);

        let Some(AvailableLiquidity {
            input: inp_bid,
            output: out_bid,
        }) = pool.available_liquidity_by_order_price(Bid(worst_bid_price))
        else {
            !panic!();
        };
        // assert!(out_ask as f64 / inp_ask as f64 >= worst_ask_price.unwrap().to_f64().unwrap());
        // assert!(out_bid as f64 / inp_bid as f64 >= worst_bid_price.unwrap().to_f64().unwrap().recip());
        assert_eq!(y_rec, 867999);
        assert_eq!(inp_ask, 999999);
        // assert_eq!(out_ask, 867999);
        //
        // assert_eq!(x_rec, 867999);
        // assert_eq!(inp_bid, 999999);
        // assert_eq!(out_bid, 867999);
    }

    #[test]
    fn available_liquidity_by_spot_price_test() {
        let lp_fee_num = 1212;
        let tr_fee_num = 30000;
        let reserves_x = 3_000_000_000_000;
        let reserves_y = 1_000_000_000_000;

        let pool = gen_ada_token_pool(
            reserves_x,
            0,
            reserves_y,
            0,
            lp_fee_num,
            lp_fee_num,
            tr_fee_num,
            0,
            0,
            1000 * 16,
        );
        let spot = pool.static_price().unwrap().to_f64().unwrap();
        let base_in = 10_000_000_000;
        // ASK:
        let Next::Succ(pool1) = pool.swap(OnSide::Ask(base_in)) else {
            panic!()
        };
        let y_rec = pool.reserves_y.untag() - pool1.reserves_y.untag();
        println!("spot: {}", spot.to_f64().unwrap());

        let final_ask_spot = pool1.static_price();
        println!("final_ask_spot: {}", final_ask_spot.unwrap().to_f64().unwrap());
        let Some(AvailableLiquidity {
            input: inp_ask,
            output: out_ask,
        }) = pool.available_liquidity_by_spot_price(final_ask_spot.into())
        else {
            !panic!();
        };
        let Next::Succ(pool11) = pool.swap(OnSide::Ask(inp_ask)) else {
            panic!()
        };
        let real_ask_spot = pool11.static_price();
        println!("real_ask_spot: {}", real_ask_spot.unwrap().to_f64().unwrap());

        // BID:
        let Next::Succ(pool2) = pool.swap(OnSide::Bid(base_in)) else {
            panic!()
        };
        let x_rec = pool.reserves_x.untag() - pool2.reserves_x.untag();
        let final_bid_spot = pool2.static_price();

        let Some(AvailableLiquidity {
            input: inp_bid,
            output: out_bid,
        }) = pool.available_liquidity_by_spot_price(final_bid_spot)
        else {
            !panic!();
        };
        // assert!(out_ask as f64 / inp_ask as f64 >= worst_ask_price.unwrap().to_f64().unwrap());
        // assert!(out_bid as f64 / inp_bid as f64 >= worst_bid_price.unwrap().to_f64().unwrap().recip());
        // println!("y_rec: {}", y_rec);
        // println!("out_ask: {}", out_ask);
        // println!("in_bid: {}", inp_bid);

        println!("x_rec: {}", x_rec);
        println!("inp_bid: {}", inp_bid);

        // assert_eq!(inp_ask, base_in);
        // assert_eq!(out_ask, 867999);
        //
        // assert_eq!(x_rec, 867999);
        assert_eq!(inp_bid, 999999);
        // assert_eq!(out_bid, 867999);
    }
    #[test]
    fn safe_pool_ratio_test() {
        // This test calculates the asset ratio in which one of the assets is available for sale
        // at a price no better than the specified one.

        // Set initial pool state (should be noticeably disbalanced to the lower price than the target):
        let pool = gen_ada_token_pool(590973688007, 6, 984457846729, 6, 100, 100, 0, 0, 0, 200 * 16);

        // Let's say we want to calculate pool in which an asset Y is available for less than 1X (including fees):
        let x_calc_value = pool.reserves_x.untag() * pool.multiplier_x;
        let x_calc = U512::from(x_calc_value);
        let y_calc = U512::from(pool.reserves_y.untag() * pool.multiplier_y);
        let an2n_value = pool.an2n;
        let an2n = U512::from(pool.an2n);

        let d = calculate_invariant(&x_calc, &y_calc, &an2n).unwrap();

        // Set the target price:
        let target_spot = 1f64;

        let total_fee = pool.lp_fee_y.to_f64().unwrap();

        // Calculate the safe ratio:
        // NB: "alpha" can be > 1 only if target_spot == 1. Recommended to set alpha in range 1 <= alpha <= ampl_coeff.
        let (x_safe, y_safe) = calculate_safe_price_ratio_x_y_swap(
            &target_spot,
            &d.as_u128(),
            &x_calc_value,
            &an2n_value,
            &total_fee,
            &200,
        );
        assert_eq!(x_safe, 640474309905);
        assert_eq!(y_safe, 934897647163);
    }

    #[test]
    fn safe_pool_ratio_validation_test() {
        // This test calculates the asset ratio in which one of the assets is available for sale
        // at a price no better than the specified one.

        // Set initial pool state.
        // Assume x reserves is ADA and y reserves is OADA.
        let pool0 = gen_ada_token_pool(590973688007, 6, 984457846729, 6, 100, 100, 0, 0, 0, 200 * 16);

        let x_safe = 640474309905;
        let x_balance_swap = x_safe - pool0.reserves_x.untag();
        let Next::Succ(pool) = pool0.swap(OnSide::Ask(x_balance_swap)) else {
            panic!()
        };
        assert_eq!(pool.reserves_x.untag(), x_safe);

        // ADA -> OADA swap:
        let ada_in = 1_000_000; // 1 ADA
        let Next::Succ(result) = pool.swap(OnSide::Ask(ada_in)) else {
            panic!()
        };
        let oada_rec = pool.reserves_y.untag() - result.reserves_y.untag();
        let spot = pool.static_price().unwrap();
        assert!(spot.to_f64().unwrap() > 1f64);
        assert!(oada_rec < ada_in);
    }

    #[test]
    fn available_liquidity_precision_tolerance_test() {
        let mut rng = StdRng::seed_from_u64(42);
        for _ in 0..1 {
            let reserves_x = 1_000_000_000;
            let reserves_y = 1_000_000_000;
            let in_amount = 1_000_00;
            let fee_num = rng.gen_range(1..50000);
            let treasury_fee_num = rng.gen_range(1..9900);
            let treasury_x = rng.gen_range(1..reserves_x / 10);
            let treasury_y = rng.gen_range(1..reserves_y / 10);

            let pool = gen_ada_token_pool(
                reserves_x,
                0,
                reserves_y,
                0,
                fee_num,
                fee_num,
                treasury_fee_num,
                treasury_x,
                treasury_y,
                200 * 16,
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
}
