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
use num_rational::Ratio;
use num_traits::{CheckedAdd, CheckedSub, Pow, ToPrimitive};
use primitive_types::U512;
use void::Void;

use bloom_offchain::execution_engine::liquidity_book::core::Next;
use bloom_offchain::execution_engine::liquidity_book::market_maker::{
    AbsoluteReserves, MakerBehavior, MarketMaker, PoolQuality, SpotPrice,
};
use bloom_offchain::execution_engine::liquidity_book::market_maker::AvailableLiquidity;
use bloom_offchain::execution_engine::liquidity_book::side::{OnSide, Side};
use bloom_offchain::execution_engine::liquidity_book::types::AbsolutePrice;
use spectrum_cardano_lib::{TaggedAmount, TaggedAssetClass, Token};
use spectrum_cardano_lib::AssetClass::Native;
use spectrum_cardano_lib::ex_units::ExUnits;
use spectrum_cardano_lib::plutus_data::{ConstrPlutusDataExtension, DatumExtension};
use spectrum_cardano_lib::plutus_data::{IntoPlutusData, PlutusDataExtension};
use spectrum_cardano_lib::transaction::TransactionOutputExtension;
use spectrum_cardano_lib::types::TryFromPData;
use spectrum_cardano_lib::value::ValueExtension;
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
use crate::data::PoolId;
use crate::data::redeem::ClassicalOnChainRedeem;
use crate::deployment::{DeployedScriptInfo, DeployedValidator, DeployedValidatorErased, RequiresValidator};
use crate::deployment::ProtocolValidator::{
    StableFnPoolT2T, StableFnPoolT2TExact,
};
use crate::pool_math::cfmm_math::classic_cfmm_shares_amount;
use crate::pool_math::stable_math::stable_cfmm_reward_lp;
use crate::pool_math::stable_pool_t2t_math::{
    calc_stable_swap_t2t, calc_stable_swap_t2t_exact, calculate_context_values_list, calculate_invariant,
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
    V1Exact,
}

impl StablePoolT2TVer {
    pub fn try_from_address<Ctx>(pool_addr: &Address, ctx: &Ctx) -> Option<StablePoolT2TVer>
    where
        Ctx: Has<DeployedScriptInfo<{ StableFnPoolT2TExact as u8 }>>,
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
            } else if ctx
                .select::<DeployedScriptInfo<{ StableFnPoolT2TExact as u8 }>>()
                .script_hash
                == *this_hash
            {
                return Some(StablePoolT2TVer::V1Exact);
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
    Ctx: Has<DeployedScriptInfo<{ StableFnPoolT2TExact as u8 }>> + Has<PoolValidation>,
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
            let marginal_cost = match pool_ver {
                StablePoolT2TVer::V1 => {
                    ctx.select::<DeployedScriptInfo<{ StableFnPoolT2T as u8 }>>()
                        .marginal_cost
                }
                StablePoolT2TVer::V1Exact => {
                    ctx.select::<DeployedScriptInfo<{ StableFnPoolT2TExact as u8 }>>()
                        .marginal_cost
                }
            };
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
                    marginal_cost: marginal_cost,
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
    Ctx: Has<DeployedValidator<{ StableFnPoolT2TExact as u8 }>>,
{
    fn get_validator(&self, ctx: &Ctx) -> DeployedValidatorErased {
        match self.ver {
            StablePoolT2TVer::V1 => ctx
                .select::<DeployedValidator<{ StableFnPoolT2TExact as u8 }>>()
                .erased(),
            StablePoolT2TVer::V1Exact => ctx
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
        match self.ver {
            StablePoolT2TVer::V1 => calc_stable_swap_t2t(
                self.asset_x,
                self.reserves_x - self.treasury_x,
                self.multiplier_x,
                self.reserves_y - self.treasury_y,
                self.multiplier_y,
                base_asset,
                base_amount,
                self.an2n,
            )
            .unwrap_or(TaggedAmount::new(0)),
            StablePoolT2TVer::V1Exact => calc_stable_swap_t2t_exact(
                self.asset_x,
                self.reserves_x - self.treasury_x,
                self.multiplier_x,
                self.reserves_y - self.treasury_y,
                self.multiplier_y,
                base_asset,
                base_amount,
                self.an2n,
            )
            .unwrap_or(TaggedAmount::new(0)),
        }
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
        let mut valid_treasury_fee = treasury_fee_ * f_rev >= quote_total_delta * self.treasury_fee.numer();
        let treasury_fee = {
            match self.ver {
                StablePoolT2TVer::V1Exact => {
                    if valid_treasury_fee || *self.treasury_fee.numer() == 0 {
                        treasury_fee_
                    } else {
                        while !valid_treasury_fee {
                            treasury_fee_ += 1;
                            quote_total_delta = pure_output - lp_fees - treasury_fee_;
                            valid_treasury_fee =
                                treasury_fee_ * f_rev >= quote_total_delta * self.treasury_fee.numer();
                        }
                        treasury_fee_
                    }
                }
                StablePoolT2TVer::V1 => treasury_fee_,
            }
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

        let nn = N_TRADABLE_ASSETS.pow(N_TRADABLE_ASSETS as u32);
        let ann = self.an2n / nn;
        let d = calculate_invariant(&U512::from(x_calc), &U512::from(y_calc), &U512::from(self.an2n))
            .expect(format!("Invalid pool state {:?}", self).as_str());

        let dn1 = vec![d; usize::try_from(N_TRADABLE_ASSETS + 1).unwrap()]
            .iter()
            .copied()
            .reduce(|a, b| a * b)
            .unwrap();
        let price_num = ann
            + (dn1
                / U512::from(nn)
                    .mul(U512::from(x_calc).pow(U512::from(2)))
                    .mul(U512::from(y_calc)))
            .as_u64();
        let price_denom = self.treasury_fee.denom().to_u64().unwrap()
            * (ann
                + (dn1
                    / U512::from(nn)
                        .mul(U512::from(y_calc).pow(U512::from(2)))
                        .mul(U512::from(x_calc)))
                .as_u64());

        let [base, _] = order_canonical(x, y);
        if x == base {
            let reversed_total_fee_num_x =
                self.treasury_fee.denom() - self.lp_fee_x.numer() - self.treasury_fee.numer();

            AbsolutePrice::new_unsafe(price_num * reversed_total_fee_num_x, price_denom).into()
        } else {
            let reversed_total_fee_num_y =
                self.treasury_fee.denom() - self.lp_fee_y.numer() - self.treasury_fee.numer();
            AbsolutePrice::new_unsafe(price_denom, reversed_total_fee_num_y * price_num).into()
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

        let y1 = (n_4.clone()
            * x.clone()
            * (n_4.clone() * ampl_coeff.clone() * d.clone() - n_4 * ampl_coeff.clone() * x.clone() - d)
            + k)
            / (n_32 * ampl_coeff * x.clone());

        let pure_quote_delta = y0 - y1;
        let quote_delta_ = pure_quote_delta.clone() - pure_quote_delta.clone() * total_fee_mult;
        let suppose_quote_delta = <u64>::try_from(quote_delta_.value.to_int().value()).ok()?;
        let output_amount_val = if suppose_quote_delta > y0_orig {
            y0_orig
        } else {
            suppose_quote_delta
        };
        Some(AvailableLiquidity {
            input: input_amount_val,
            output: output_amount_val,
        })
    }

    fn estimated_trade(&self, input: OnSide<u64>) -> Option<AvailableLiquidity> {
        unimplemented!()
    }

    fn is_active(&self) -> bool {
        // balance pools do not support lq bound, so
        // swaps allowed all time
        true
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
