use std::fmt::{Debug, Display, Formatter};
use std::ops::Div;

use bignumber::BigNumber;
use cml_chain::address::Address;
use cml_chain::assets::MultiAsset;
use cml_chain::auxdata::{Metadata, TransactionMetadatum};
use cml_chain::certs::StakeCredential;
use cml_chain::plutus::{ConstrPlutusData, PlutusData};
use cml_chain::transaction::{ConwayFormatTxOut, TransactionOutput};
use cml_chain::utils::BigInteger;
use cml_chain::Value;
use cml_core::serialization::RawBytesEncoding;
use cml_core::Int;
use cml_crypto::{Ed25519KeyHash, ScriptHash};
use dashu_float::DBig;
use log::trace;
use num_traits::{CheckedDiv, CheckedSub, ToPrimitive};
use type_equalities::IsEqual;
use void::Void;

use bloom_offchain::execution_engine::liquidity_book::core::Next;
use bloom_offchain::execution_engine::liquidity_book::market_maker::{
    AbsoluteReserves, AvailableLiquidity, MakerBehavior, MarketMaker, PoolQuality, SpotPrice,
};
use bloom_offchain::execution_engine::liquidity_book::side::{OnSide, Side};
use bloom_offchain::execution_engine::liquidity_book::types::AbsolutePrice;
use spectrum_cardano_lib::address::InlineCredential;
use spectrum_cardano_lib::ex_units::ExUnits;
use spectrum_cardano_lib::plutus_data::{
    ConstrPlutusDataExtension, DatumExtension, IntoPlutusData, PlutusDataExtension,
};
use spectrum_cardano_lib::transaction::TransactionOutputExtension;
use spectrum_cardano_lib::types::TryFromPData;
use spectrum_cardano_lib::value::ValueExtension;
use spectrum_cardano_lib::{TaggedAmount, TaggedAssetClass, Token};
use spectrum_offchain::domain::{Has, SeqState, Stable, Tradable};
use spectrum_offchain::ledger::{IntoLedger, TryFromLedger};

use crate::creds::OperatorCred;
use crate::data::cfmm_pool::royalty_pool::ROYALTY_DATUM_MAPPING;
use crate::data::order::{Base, PoolNft, Quote};
use crate::data::pair::{order_canonical, PairId};
use crate::data::pool::{
    ApplyOrder, CFMMPoolAction, ImmutablePoolUtxo, PoolAssetMapping, PoolValidation, Rx, Ry,
};
use crate::data::PoolId;
use crate::deployment::ProtocolValidator::{DegenQuadraticPoolV1, DegenQuadraticPoolV1T2T};
use crate::deployment::{DeployedScriptInfo, DeployedValidator, DeployedValidatorErased, RequiresValidator};
use crate::fees::FeeExtension;
use crate::handler_context::Mints;
use crate::pool_math::degen_quadratic_math::{
    degen_quadratic_output_amount, A_DENOM, B_DENOM, FULL_PERCENTILE, MAX_ALLOWED_ADA_EXTRAS_PERCENTILE,
    MIN_ADA, TOKEN_EMISSION,
};

pub struct QuadraticPoolConfig {
    pub pool_nft: TaggedAssetClass<PoolNft>,
    pub asset_x: TaggedAssetClass<Rx>,
    pub asset_y: TaggedAssetClass<Ry>,
    pub a_num: u64,
    pub b_num: u64,
    pub operator_pkh: Ed25519KeyHash,
    pub x_cap_thr: u64,
}

struct DatumMapping {
    pub pool_nft: usize,
    pub asset_x: usize,
    pub asset_y: usize,
    pub a_num: usize,
    pub b_num: usize,
    pub operator_pkh: usize,
    pub x_cap_thr: usize,
}

const DATUM_MAPPING: DatumMapping = DatumMapping {
    pool_nft: 0,
    asset_x: 1,
    asset_y: 2,
    a_num: 3,
    b_num: 4,
    operator_pkh: 5,
    x_cap_thr: 6,
};

impl TryFromPData for QuadraticPoolConfig {
    fn try_from_pd(data: PlutusData) -> Option<Self> {
        let mut cpd = data.into_constr_pd()?;
        let pool_nft = TaggedAssetClass::try_from_pd(cpd.take_field(DATUM_MAPPING.pool_nft)?)?;
        let asset_x = TaggedAssetClass::try_from_pd(cpd.take_field(DATUM_MAPPING.asset_x)?)?;
        let asset_y = TaggedAssetClass::try_from_pd(cpd.take_field(DATUM_MAPPING.asset_y)?)?;
        let a_num = cpd.take_field(DATUM_MAPPING.a_num)?.into_u64()?;
        let b_num = cpd.take_field(DATUM_MAPPING.b_num)?.into_u64()?;
        let operator_pkh =
            Ed25519KeyHash::from_raw_bytes(&*cpd.take_field(DATUM_MAPPING.operator_pkh)?.into_bytes()?)
                .ok()?;
        let ada_cap_thr = cpd.take_field(DATUM_MAPPING.x_cap_thr)?.into_u64()?;

        Some(Self {
            pool_nft,
            asset_x,
            asset_y,
            a_num,
            b_num,
            operator_pkh,
            x_cap_thr: ada_cap_thr,
        })
    }
}

const T2T_FEE_NUM: u8 = 1;
const T2T_FEE_DENOM: u8 = 100;

pub struct QuadraticPoolT2TConfig {
    pub pool_nft: TaggedAssetClass<PoolNft>,
    pub asset_x: TaggedAssetClass<Rx>,
    pub asset_y: TaggedAssetClass<Ry>,
    pub a_num: u64,
    pub b_num: u64,
    pub operator_pkh: Ed25519KeyHash,
    pub x_cap_thr: u64,
    pub accumulated_x_fee: u64,
}

struct DatumT2TMapping {
    pub pool_nft: usize,
    pub asset_x: usize,
    pub asset_y: usize,
    pub a_num: usize,
    pub b_num: usize,
    pub operator_pkh: usize,
    pub x_cap_thr: usize,
    pub accumulated_x_fee: usize,
}

const DATUM_T2T_MAPPING: DatumT2TMapping = DatumT2TMapping {
    pool_nft: 0,
    asset_x: 1,
    asset_y: 2,
    a_num: 3,
    b_num: 4,
    operator_pkh: 5,
    x_cap_thr: 6,
    accumulated_x_fee: 7,
};

impl TryFromPData for QuadraticPoolT2TConfig {
    fn try_from_pd(data: PlutusData) -> Option<Self> {
        let mut cpd = data.into_constr_pd()?;
        let pool_nft = TaggedAssetClass::try_from_pd(cpd.take_field(DATUM_T2T_MAPPING.pool_nft)?)?;
        let asset_x = TaggedAssetClass::try_from_pd(cpd.take_field(DATUM_T2T_MAPPING.asset_x)?)?;
        let asset_y = TaggedAssetClass::try_from_pd(cpd.take_field(DATUM_T2T_MAPPING.asset_y)?)?;
        let a_num = cpd.take_field(DATUM_T2T_MAPPING.a_num)?.into_u64()?;
        let b_num = cpd.take_field(DATUM_T2T_MAPPING.b_num)?.into_u64()?;
        let operator_pkh =
            Ed25519KeyHash::from_raw_bytes(&*cpd.take_field(DATUM_T2T_MAPPING.operator_pkh)?.into_bytes()?)
                .ok()?;
        let ada_cap_thr = cpd.take_field(DATUM_T2T_MAPPING.x_cap_thr)?.into_u64()?;
        let x_fee = cpd.take_field(DATUM_T2T_MAPPING.accumulated_x_fee)?.into_u64()?;

        Some(Self {
            pool_nft,
            asset_x,
            asset_y,
            a_num,
            b_num,
            operator_pkh,
            x_cap_thr: ada_cap_thr,
            accumulated_x_fee: x_fee,
        })
    }
}

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub enum QuadraticPoolVer {
    V1,
    V1T2T,
}

impl QuadraticPoolVer {
    pub fn try_from_address<Ctx>(pool_addr: &Address, ctx: &Ctx) -> Option<QuadraticPoolVer>
    where
        Ctx: Has<DeployedScriptInfo<{ DegenQuadraticPoolV1 as u8 }>>
            + Has<DeployedScriptInfo<{ DegenQuadraticPoolV1T2T as u8 }>>
            + Has<PoolValidation>,
    {
        let maybe_hash = pool_addr.payment_cred().and_then(|c| match c {
            StakeCredential::PubKey { .. } => None,
            StakeCredential::Script { hash, .. } => Some(hash),
        });
        if let Some(this_hash) = maybe_hash {
            if ctx
                .select::<DeployedScriptInfo<{ DegenQuadraticPoolV1 as u8 }>>()
                .script_hash
                == *this_hash
            {
                return Some(QuadraticPoolVer::V1);
            } else if ctx
                .select::<DeployedScriptInfo<{ DegenQuadraticPoolV1T2T as u8 }>>()
                .script_hash
                == *this_hash
            {
                return Some(QuadraticPoolVer::V1T2T);
            }
        }
        None
    }
}

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub struct QuadraticPool {
    pub id: PoolId,
    pub reserves_x: TaggedAmount<Rx>,
    pub reserves_y: TaggedAmount<Ry>,
    pub asset_x: TaggedAssetClass<Rx>,
    pub asset_y: TaggedAssetClass<Ry>,
    pub a_num: u64,
    pub b_num: u64,
    // for n2t - lovelace, for t2t - token
    pub cap_thr: u64,
    pub ver: QuadraticPoolVer,
    pub accumulated_x_fee: TaggedAmount<Rx>,
    pub x_fee_num: u8,
    pub marginal_cost: ExUnits,
    pub bounds: PoolValidation,
    pub virgin: bool,
    pub capped: bool,
}

impl Display for QuadraticPool {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(&*format!(
            "QuadraticPool(id: {}, static_price: {}, quality: {}, accumulated_x_fee: {})",
            self.id,
            self.static_price(),
            self.quality(),
            self.accumulated_x_fee
        ))
    }
}

impl QuadraticPool {
    /// Computes the effective amount of tokens to be swapped, after applying the T2T fee ratio:
    /// `input * (T2T_FEE_DENOM - x_fee_num) / T2T_FEE_DENOM`.
    ///
    /// Due to integer division, the result is adjusted by subtracting:
    /// - **2** if the division is exact (no remainder)
    /// - **1** otherwise (one unit is already lost to floor division)
    ///
    /// This correction is required to satisfy the quadratic t2t contract, which expects:
    /// - Rounding losses to be explicitly accounted for
    /// - Accumulated fee to be incremented on every swap
    pub fn adjusted_x_input(x_fee_num: u8, input: u64) -> u64 {
        let input_num = T2T_FEE_DENOM as u64 - x_fee_num as u64;
        let raw_input = input * input_num / T2T_FEE_DENOM as u64;
        let input_correction = if input * input_num % T2T_FEE_DENOM as u64 == 0 {
            2
        } else {
            // in this case we already "remove" another token due to rounding to bottom
            1
        };

        raw_input - input_correction
    }

    /// Computes the effective amount of X_TOKEN the user will receive, adjusted for T2T fee.
    ///
    /// To satisfy the quadratic pool t2t contract, we subtract:
    /// - **1** if the division is exact (no remainder)
    /// - **0** otherwise (flooring already reduces the result)
    pub fn adjust_x_output(x_fee_num: u8, output: u64) -> u64 {
        let output_num = T2T_FEE_DENOM as u64 - x_fee_num as u64;
        let raw_output = output * output_num / T2T_FEE_DENOM as u64;
        let output_correction = if output * output_num % T2T_FEE_DENOM as u64 == 0 {
            1
        } else {
            0
        };
        raw_output - output_correction
    }

    /// Calculates the fee amount in X_TOKEN to be added to the pool's accumulated fee,
    /// based on the user's input.
    ///
    /// Uses the ratio:
    /// `input * x_fee_num / T2T_FEE_DENOM`
    ///
    /// To comply with the quadratic pool t2t contract, the fee must always round up:
    /// - If the division is exact, return the result as-is.
    /// - If it has a remainder, add 1 to avoid undercharging.
    pub fn adjust_x_fee(x_fee_num: u8, input: u64) -> u64 {
        let raw_fee = input * x_fee_num as u64 / T2T_FEE_DENOM as u64;
        let fee_correction = if input * x_fee_num as u64 % T2T_FEE_DENOM as u64 == 0 {
            0
        } else {
            1
        };
        raw_fee + fee_correction
    }

    pub fn asset_mapping(&self, side: Side) -> PoolAssetMapping {
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
    fn create_redeemer(pool_in_idx: u64, pool_out_idx: u64) -> PlutusData {
        let self_in_idx_pd = PlutusData::Integer(BigInteger::from(pool_in_idx));
        let self_out_idx_pd = PlutusData::Integer(BigInteger::from(pool_out_idx));
        let amm_action = PlutusData::ConstrPlutusData(ConstrPlutusData::new(0, Vec::from([])));

        PlutusData::ConstrPlutusData(ConstrPlutusData::new(
            0,
            Vec::from([self_in_idx_pd, self_out_idx_pd, amm_action]),
        ))
    }
}

pub struct QuadraticPoolRedeemer {
    pub pool_input_index: u64,
    pub pool_output_index: u64,
    pub action: CFMMPoolAction,
}

impl QuadraticPoolRedeemer {
    pub fn to_plutus_data(self) -> PlutusData {
        QuadraticPool::create_redeemer(self.pool_input_index, self.pool_output_index)
    }
}

pub trait AMMOps {
    fn output_amount(
        &self,
        base_asset: TaggedAssetClass<Base>,
        base_amount: TaggedAmount<Base>,
    ) -> TaggedAmount<Quote>;
}

impl AMMOps for QuadraticPool {
    fn output_amount(
        &self,
        base_asset: TaggedAssetClass<Base>,
        base_amount: TaggedAmount<Base>,
    ) -> TaggedAmount<Quote> {
        degen_quadratic_output_amount(
            self.asset_x,
            self.reserves_y,
            base_asset,
            base_amount,
            self.a_num,
            self.b_num,
            self.accumulated_x_fee,
        )
    }
}

impl<Ctx> RequiresValidator<Ctx> for QuadraticPool
where
    Ctx: Has<DeployedValidator<{ DegenQuadraticPoolV1 as u8 }>>
        + Has<DeployedValidator<{ DegenQuadraticPoolV1T2T as u8 }>>,
{
    fn get_validator(&self, ctx: &Ctx) -> DeployedValidatorErased {
        match self.ver {
            QuadraticPoolVer::V1 => ctx
                .select::<DeployedValidator<{ DegenQuadraticPoolV1 as u8 }>>()
                .erased(),
            QuadraticPoolVer::V1T2T => ctx
                .select::<DeployedValidator<{ DegenQuadraticPoolV1T2T as u8 }>>()
                .erased(),
        }
    }
}

impl MakerBehavior for QuadraticPool {
    fn swap(mut self, input: OnSide<u64>) -> Next<Self, Void> {
        let x = self.asset_x.untag();
        let y = self.asset_y.untag();
        let [base, quote] = order_canonical(x, y);

        match self.ver {
            QuadraticPoolVer::V1 => {
                let output_candidate = match input {
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
                let safe_output = match input {
                    OnSide::Bid(_) => {
                        if output_candidate + MIN_ADA <= *base_reserves {
                            output_candidate
                        } else {
                            *base_reserves
                        }
                    }

                    OnSide::Ask(_) => {
                        if output_candidate <= *quote_reserves {
                            output_candidate
                        } else {
                            *quote_reserves
                        }
                    }
                };
                match input {
                    OnSide::Bid(input) => {
                        // A user bid means that they wish to buy the base asset for the quote asset, hence
                        // pool reserves of base decreases while reserves of quote increase.
                        *quote_reserves += input;
                        *base_reserves -= safe_output;
                    }
                    OnSide::Ask(input) => {
                        // User ask is the opposite; sell the base asset for the quote asset.
                        *base_reserves += input;
                        *quote_reserves -= safe_output;
                    }
                }
                Next::Succ(self)
            }
            // The T2T quadratic pool enforces specific rules for calculating input, output, and accumulated fee.
            // These rules depend on the swap direction and are required to ensure on-chain contract validation passes.
            //
            // - When swapping X_TOKEN → Y_TOKEN, the user's X_TOKEN input must be reduced.
            // - When swapping Y_TOKEN → X_TOKEN, the user's X_TOKEN output must be reduced.
            //
            // These adjustments are necessary to account for rounding behavior and to maintain
            // consistency with how the quadratic pool contract performs swap validation on-chain.
            QuadraticPoolVer::V1T2T => {
                let adjusted_input = match input {
                    OnSide::Bid(_) if x == quote => {
                        QuadraticPool::adjusted_x_input(self.x_fee_num, input.unwrap())
                    }
                    OnSide::Ask(_) if x == base => {
                        QuadraticPool::adjusted_x_input(self.x_fee_num, input.unwrap())
                    }
                    _ => input.unwrap(),
                };

                let output_candidate = match input {
                    OnSide::Ask(_) => self
                        .output_amount(TaggedAssetClass::new(base), TaggedAmount::new(adjusted_input))
                        .untag(),
                    OnSide::Bid(_) => self
                        .output_amount(TaggedAssetClass::new(quote), TaggedAmount::new(adjusted_input))
                        .untag(),
                };
                let (base_reserves, quote_reserves) = if x == base {
                    (self.reserves_x.as_mut(), self.reserves_y.as_mut())
                } else {
                    (self.reserves_y.as_mut(), self.reserves_x.as_mut())
                };
                let safe_output = match input {
                    OnSide::Bid(_) => {
                        if (self.ver == QuadraticPoolVer::V1 && output_candidate + MIN_ADA <= *base_reserves)
                            || (self.ver == QuadraticPoolVer::V1T2T)
                        {
                            output_candidate
                        } else {
                            *base_reserves
                        }
                    }

                    OnSide::Ask(_) => {
                        if output_candidate <= *quote_reserves {
                            output_candidate
                        } else {
                            *quote_reserves
                        }
                    }
                };
                // update accumulated_x_fee
                let additional_fee = match input {
                    OnSide::Bid(_) if x == quote => {
                        TaggedAmount::new(QuadraticPool::adjust_x_fee(self.x_fee_num, input.unwrap()))
                    }
                    // we should get fee from input
                    OnSide::Ask(_) if x == base => {
                        TaggedAmount::new(QuadraticPool::adjust_x_fee(self.x_fee_num, input.unwrap()))
                    }
                    // we should get fee from output
                    _ => TaggedAmount::new(QuadraticPool::adjust_x_fee(self.x_fee_num, safe_output)),
                };
                self.accumulated_x_fee += additional_fee;
                match input {
                    OnSide::Bid(input) if x == quote => {
                        *quote_reserves += input;
                        *base_reserves -= safe_output;
                    }
                    OnSide::Bid(input) => {
                        *quote_reserves += input;
                        *base_reserves -= QuadraticPool::adjust_x_output(self.x_fee_num, safe_output);
                    }
                    OnSide::Ask(input) if x == base => {
                        *base_reserves += input;
                        *quote_reserves -= safe_output;
                    }
                    OnSide::Ask(input) => {
                        *base_reserves += input;
                        *quote_reserves -= QuadraticPool::adjust_x_output(self.x_fee_num, safe_output);
                    }
                }
                Next::Succ(self)
            }
        }
    }
}

impl MarketMaker for QuadraticPool {
    type U = ExUnits;

    fn static_price(&self) -> SpotPrice {
        let x = self.asset_x.untag();
        let y = self.asset_y.untag();
        let [base, _] = order_canonical(x, y);
        let supply = (TOKEN_EMISSION - self.reserves_y.untag()) as u128;
        let price_num = (supply * supply * self.a_num as u128) * B_DENOM + A_DENOM * self.b_num as u128;
        let price_denom = A_DENOM * B_DENOM;
        if x == base {
            AbsolutePrice::new_raw(price_denom, price_num).into()
        } else {
            AbsolutePrice::new_raw(price_num, price_denom).into()
        }
    }

    fn real_price(&self, input: OnSide<u64>) -> Option<AbsolutePrice> {
        let x = self.asset_x.untag();
        let y = self.asset_y.untag();
        let [base, quote] = order_canonical(x, y);
        // Verification that current swap wouldn't overfill ada_cap_thr
        let (base, quote) = match input {
            OnSide::Bid(input) => (
                self.output_amount(TaggedAssetClass::new(quote), TaggedAmount::new(input))
                    .untag(),
                input,
            ),
            OnSide::Ask(input)
                if self.reserves_x.untag() + input - self.accumulated_x_fee.untag() <= self.cap_thr =>
            {
                (
                    input,
                    self.output_amount(TaggedAssetClass::new(base), TaggedAmount::new(input))
                        .untag(),
                )
            }
            // Swap will overfill ada_cap_thr with corresponding input. So, we couldn't execute it
            OnSide::Ask(_) => return None,
        };
        AbsolutePrice::new(quote, base)
    }
    fn quality(&self) -> PoolQuality {
        let reserves_x = self.reserves_x.untag();
        let quality = if self.asset_x.is_native() && reserves_x >= MIN_ADA {
            reserves_x - MIN_ADA
        } else if !self.asset_x.is_native() {
            reserves_x - self.accumulated_x_fee.untag()
        } else {
            0
        };
        PoolQuality::from(quality as u128)
    }

    fn marginal_cost_hint(&self) -> Self::U {
        self.marginal_cost
    }

    fn is_active(&self) -> bool {
        if self.ver == QuadraticPoolVer::V1 {
            self.reserves_x.untag()
                < self.cap_thr * (MAX_ALLOWED_ADA_EXTRAS_PERCENTILE + FULL_PERCENTILE) / FULL_PERCENTILE
        } else {
            (self.reserves_x.untag() - self.accumulated_x_fee.untag())
                < self.cap_thr * (MAX_ALLOWED_ADA_EXTRAS_PERCENTILE + FULL_PERCENTILE) / FULL_PERCENTILE
        }
    }

    fn liquidity(&self) -> AbsoluteReserves {
        let x = self.asset_x.untag();
        let y = self.asset_y.untag();
        let [base, _] = order_canonical(x, y);
        let reserves_x_candidate = self.reserves_x.untag();
        let available_reserves_x = if self.asset_x.is_native() && reserves_x_candidate >= MIN_ADA {
            reserves_x_candidate - MIN_ADA
        } else if !self.asset_x.is_native() {
            reserves_x_candidate
        } else {
            0
        };
        if base == x {
            AbsoluteReserves {
                base: available_reserves_x,
                quote: self.reserves_y.untag(),
            }
        } else {
            AbsoluteReserves {
                base: self.reserves_y.untag(),
                quote: available_reserves_x,
            }
        }
    }

    fn available_liquidity_on_side(&self, worst_price: OnSide<AbsolutePrice>) -> Option<AvailableLiquidity> {
        const BN_ZERO: BigNumber = BigNumber { value: DBig::ZERO };
        const MAX_EXCESS_PERC: u64 = 1;
        const PERC: u64 = 100;

        let n_27 = BigNumber::from(27f64);
        let n_2916 = BigNumber::from(2916f64);

        let sqrt_degree = BigNumber::from(0.5f64);
        let cbrt_degree = BigNumber::from(1f64 / 3f64);

        // For Degen pool quote asset is always x.
        let x0_val = if self.asset_x.is_native() {
            (self.reserves_x.untag() - MIN_ADA) as f64
        } else {
            (self.reserves_x.untag() - self.accumulated_x_fee.untag()) as f64
        };

        let [base, _] = order_canonical(self.asset_x.untag(), self.asset_y.untag());
        let (degen_side, degen_canonical) = match worst_price {
            OnSide::Bid(_) if base == self.asset_x.untag() => (Side::Bid, true),
            OnSide::Bid(_) => (Side::Ask, false),
            OnSide::Ask(_) if base == self.asset_x.untag() => (Side::Ask, true),
            OnSide::Ask(_) => (Side::Bid, false),
        };

        let x0 = BigNumber::from(x0_val);
        let supply_y0 = BigNumber::from((TOKEN_EMISSION - self.reserves_y.untag()) as f64);
        let supply_y0_x3 = supply_y0.clone() * supply_y0.clone() * supply_y0.clone();

        let a = BigNumber::from(self.a_num as f64) / BigNumber::from(A_DENOM as f64);
        let a_x2 = a.clone() * a.clone();
        let a_x3 = a.clone() * a_x2.clone();
        let b = BigNumber::from(self.b_num as f64 / B_DENOM as f64);
        let b_x2 = b.clone() * b.clone();
        let b_x3 = b.clone() * b_x2.clone();

        let mut counter = 0usize;

        let max_token_x_qty = if self.asset_x.is_native() {
            BigNumber::from(((self.cap_thr - MIN_ADA) * (PERC + MAX_EXCESS_PERC) / PERC) as f64)
        } else {
            BigNumber::from((self.cap_thr * (PERC + MAX_EXCESS_PERC) / PERC) as f64)
        };
        let min_ada = BigNumber::from(MIN_ADA as f64);
        if self.asset_x.is_native() && x0_val >= (self.cap_thr - MIN_ADA) as f64 {
            return None;
        } else if x0_val >= self.cap_thr as f64 {
            return None;
        };

        let n_0 = BigNumber::from(0f64);
        let n_1_minus = BigNumber::from(-1f64);
        let n_1 = BigNumber::from(1f64);
        let n_2 = BigNumber::from(2f64);

        let mut err = n_2.clone();
        let price = worst_price.unwrap();
        let price_f64 = BigNumber::from(*price.numer() as f64) / BigNumber::from(*price.denom() as f64);
        let price_f64_rev = BigNumber::from(*price.denom() as f64) / BigNumber::from(*price.numer() as f64);

        let (output_amount, input_amount) = match degen_side {
            Side::Ask => {
                const COEFF_DENOM: u64 = 100000000000000;
                const COEFF_1_NUM: u64 = 26456684199470;
                const COEFF_0_NUM: u64 = 377976314968462;

                let p = if degen_canonical { price_f64 } else { price_f64_rev };

                let token_x_delta = if self.asset_x.is_native() {
                    self.cap_thr - x0_val as u64 - MIN_ADA
                } else {
                    self.cap_thr - x0_val as u64
                };
                let token_delta = self
                    .output_amount(
                        TaggedAssetClass::new(self.asset_x.into()),
                        TaggedAmount::new(token_x_delta),
                    )
                    .untag();
                let bound_price = BigNumber::from(token_delta as f64 / token_x_delta as f64);
                if p.value < bound_price.value {
                    return Some(AvailableLiquidity {
                        input: token_x_delta,
                        output: token_delta,
                    });
                };
                let n_81 = BigNumber::from(81f64);

                let coeff_denom = BigNumber::from(COEFF_DENOM as f64);
                let coeff_0 = BigNumber::from(COEFF_0_NUM as f64) / coeff_denom.clone();
                let coeff_1 = BigNumber::from(COEFF_1_NUM as f64) / coeff_denom;

                let mut x1 = if self.asset_x.is_native() {
                    BigNumber::from((self.cap_thr - MIN_ADA) as f64)
                } else {
                    BigNumber::from(self.cap_thr as f64)
                };

                while ((err.value.clone() >= n_1.value.clone())
                    || (err.value.clone() <= n_1_minus.value.clone()))
                    && counter < 255
                {
                    let a_coeff = n_27.clone() * a_x3.clone() * supply_y0_x3.clone()
                        + n_81.clone() * a_x2.clone() * b.clone() * supply_y0.clone()
                        + n_81.clone() * a_x2.clone() * (x1.clone() - x0.clone());
                    let a_coeff_x2 = a_coeff.clone() * a_coeff.clone();
                    let b_coeff = a_coeff.clone()
                        + (n_2916.clone() * a_x3.clone() * b_x3.clone() + a_coeff_x2.clone())
                            .pow(&sqrt_degree.clone());
                    let b_coeff_1_3 = b_coeff.pow(&cbrt_degree.clone());
                    let b_coeff_2_3 = b_coeff_1_3.clone() * b_coeff_1_3.clone();
                    let b_coeff_4_3 = b_coeff_2_3.clone() * b_coeff_2_3.clone();
                    let c_coeff = n_27.clone() * a_x2.clone() * a_coeff
                        / (n_2916.clone() * a_x3.clone() * b_x3.clone() + a_coeff_x2)
                            .pow(&sqrt_degree.clone())
                        + n_27.clone() * a_x2.clone();
                    let add_num = (coeff_1.clone() * b_coeff_1_3.clone()).div(a.clone())
                        - (coeff_0.clone() * b.clone()).div(b_coeff_1_3)
                        - p.clone() * (x1.clone() - x0.clone())
                        - supply_y0.clone();
                    let add_denom = (coeff_0.clone() * b.clone() * c_coeff.clone()).div(b_coeff_4_3)
                        - p.clone()
                        + (coeff_1.clone() * c_coeff.clone()).div(a.clone() * b_coeff_2_3);
                    let additional = add_num.div(add_denom);
                    let x_new = x1.clone() - additional.clone();
                    if x_new.value.clone() < max_token_x_qty.value.clone()
                        && ((self.asset_x.is_native() && x_new.value.clone() > min_ada.value.clone())
                            || !self.asset_x.is_native())
                    {
                        x1 = x1.clone() - additional.clone()
                    } else {
                        let token_x_delta = if self.asset_x.is_native() {
                            self.cap_thr - x0_val as u64 - MIN_ADA
                        } else {
                            self.cap_thr - x0_val as u64
                        };
                        let token_delta = self
                            .output_amount(
                                TaggedAssetClass::new(self.asset_x.into()),
                                TaggedAmount::new(token_x_delta),
                            )
                            .untag();
                        return Some(AvailableLiquidity {
                            input: token_x_delta,
                            output: token_delta,
                        });
                    }
                    err = additional.clone();
                    counter += 1;
                }
                let delta_x = x1 - x0;
                let delta_y = delta_x.clone() * p;
                (delta_y, delta_x)
            }
            Side::Bid => {
                let n_3 = BigNumber::from(3f64);
                let n_4 = BigNumber::from(4f64);
                let n_6 = BigNumber::from(6f64);
                let n_18 = BigNumber::from(18f64);
                let n_243 = BigNumber::from(243f64);
                let n_729 = BigNumber::from(729f64);

                let mut counter = 0usize;
                let p = if degen_canonical { price_f64_rev } else { price_f64 };
                let bound_price =
                    n_1.clone() / (a.clone() * BigNumber::from(TOKEN_EMISSION as f64).pow(&n_2) + b.clone());

                if p.value < bound_price.value {
                    let supply_y0_val = supply_y0.value.to_f64().value() as u64;
                    let x_val = if supply_y0_val > 0u64 { x0_val } else { 0f64 };
                    return Some(AvailableLiquidity {
                        output: x_val as u64,
                        input: supply_y0_val,
                    });
                };
                let mut x1 = BN_ZERO;

                while ((err.value.clone() >= n_1.value.clone())
                    || (err.value.clone() <= n_1_minus.value.clone()))
                    && counter < 255
                {
                    let a_coeff = n_27.clone()
                        * (n_3.clone() * x0.clone()
                            - a.clone() * supply_y0_x3.clone()
                            - n_3.clone() * b.clone() * supply_y0.clone()
                            - n_3.clone() * x1.clone())
                        / (n_2.clone() * a.clone());
                    let b_coeff_base = (n_3.clone() * x0.clone()
                        - a.clone() * supply_y0_x3.clone()
                        - n_3.clone() * b.clone() * supply_y0.clone()
                        - n_3.clone() * x1.clone());
                    let b_coeff_base_ = (n_729.clone() * b_coeff_base.clone() * b_coeff_base.clone())
                        .div(a_x2.clone())
                        + (n_2916.clone() * b_x3.clone()).div(a_x3.clone());

                    let b_coeff = b_coeff_base_.clone().pow(&sqrt_degree.clone());
                    let c_coeff = b_coeff.clone() / n_2.clone() + a_coeff.clone();
                    let c_coeff_1_3 = c_coeff.pow(&cbrt_degree.clone());
                    let c_coeff_2_3 = c_coeff_1_3.clone() * c_coeff_1_3.clone();
                    let c_coeff_4_3 = c_coeff_2_3.clone() * c_coeff_2_3.clone();

                    let d_coeff = n_27.clone() / (n_2.clone() * a.clone())
                        - n_243.clone()
                            * (n_6.clone() * a.clone() * supply_y0_x3.clone()
                                + n_18.clone() * b.clone() * supply_y0.clone()
                                - n_18.clone() * x0.clone()
                                + n_18.clone() * x1.clone())
                            / (n_4.clone() * a_x2.clone() * b_coeff);
                    let add_num = supply_y0.clone() - p.clone() * (x0.clone() - x1.clone())
                        + c_coeff_1_3.clone() / n_3.clone()
                        - n_3.clone() * b.clone() / (a.clone() * c_coeff_1_3);
                    let add_denom = p.clone()
                        - d_coeff.clone() / (n_3.clone() * c_coeff_2_3)
                        - n_3.clone() * b.clone() * d_coeff / (a.clone() * c_coeff_4_3);
                    let additional = add_num.div(add_denom);
                    let x_new = x1.clone() - additional.clone();
                    if x_new.value.clone() < max_token_x_qty.value.clone()
                        && ((self.asset_x.is_native() && x_new.value.clone() > min_ada.value.clone())
                            || !self.asset_x.is_native())
                        && x_new.value.round().clone() != x0.value.clone()
                    {
                        x1 = x1.clone() - additional.clone()
                    } else {
                        let supply_y0_val = supply_y0.value.to_f64().value() as u64;
                        let x_val = if supply_y0_val > 0u64 { x0_val } else { 0f64 };
                        return Some(AvailableLiquidity {
                            output: x_val as u64,
                            input: supply_y0.value.to_f64().value() as u64,
                        });
                    }
                    err = if x0.value.clone() > n_0.value.clone() {
                        additional.clone()
                    } else {
                        n_2.clone()
                    };
                    counter += 1;
                }
                let delta_x = x0 - x1;
                let delta_y = delta_x.clone() * p;
                (delta_x, delta_y)
            }
        };
        Some(AvailableLiquidity {
            input: input_amount.value.to_f64().value() as u64,
            output: output_amount.value.to_f64().value() as u64,
        })
    }

    fn estimated_trade(&self, input: OnSide<u64>) -> Option<AvailableLiquidity> {
        const MAX_EXCESS_PERC: u64 = 5;
        const PERC: u64 = 1000;

        let (input_amount, output_amount) = match input {
            OnSide::Bid(input) => (
                input,
                self.output_amount(
                    TaggedAssetClass::new(self.asset_y.into()),
                    TaggedAmount::new(input),
                )
                .untag(),
            ),
            OnSide::Ask(input_candidate) => {
                let token_x_reserves = self.reserves_x.untag() - self.accumulated_x_fee.untag();
                let max_ada_required = self.cap_thr * (PERC + MAX_EXCESS_PERC) / PERC - token_x_reserves;
                let max_token_available = self
                    .output_amount(
                        TaggedAssetClass::new(self.asset_x.into()),
                        TaggedAmount::new(max_ada_required),
                    )
                    .untag();

                let output_candidate = self
                    .output_amount(
                        TaggedAssetClass::new(self.asset_x.into()),
                        TaggedAmount::new(input_candidate),
                    )
                    .untag();
                let (input_final, output_final) = if output_candidate <= max_token_available {
                    (input_candidate, output_candidate)
                } else {
                    (max_ada_required, max_token_available)
                };
                (input_final, output_final)
            }
        };

        Some(AvailableLiquidity {
            input: input_amount,
            output: output_amount,
        })
    }
}

impl Has<QuadraticPoolVer> for QuadraticPool {
    fn select<U: IsEqual<QuadraticPoolVer>>(&self) -> QuadraticPoolVer {
        self.ver
    }
}

impl Stable for QuadraticPool {
    type StableId = Token;
    fn stable_id(&self) -> Self::StableId {
        self.id.into()
    }
    fn is_quasi_permanent(&self) -> bool {
        true
    }
}

impl SeqState for QuadraticPool {
    fn is_initial(&self) -> bool {
        self.virgin
    }
}

impl Tradable for QuadraticPool {
    type PairId = PairId;
    fn pair_id(&self) -> Self::PairId {
        PairId::canonical(self.asset_x.untag(), self.asset_y.untag())
    }
}

const CAP_META_KEY: u64 = 7;
const CAP_META_VALUE: u64 = 1;

fn is_capped<C: Has<Option<Metadata>>>(cx: &C) -> bool {
    cx.select::<Option<Metadata>>()
        .and_then(|meta| {
            meta.get(CAP_META_KEY).map(|v| match v {
                TransactionMetadatum::Int(Int::Uint { value, .. }) => *value == CAP_META_VALUE,
                _ => false,
            })
        })
        .unwrap_or(false)
}

impl<Ctx> TryFromLedger<TransactionOutput, Ctx> for QuadraticPool
where
    Ctx: Has<DeployedScriptInfo<{ DegenQuadraticPoolV1 as u8 }>>
        + Has<DeployedScriptInfo<{ DegenQuadraticPoolV1T2T as u8 }>>
        + Has<PoolValidation>
        + Has<OperatorCred>
        + Has<Option<Mints>>
        + Has<Option<Metadata>>,
{
    fn try_from_ledger(repr: &TransactionOutput, ctx: &Ctx) -> Option<Self> {
        if let Some(pool_ver) = QuadraticPoolVer::try_from_address(repr.address(), ctx) {
            let value = repr.value();
            let pd = repr.datum().clone()?.into_pd()?;
            let bounds = ctx.select::<PoolValidation>();
            let marginal_cost = if pool_ver == QuadraticPoolVer::V1 {
                ctx.select::<DeployedScriptInfo<{ DegenQuadraticPoolV1 as u8 }>>()
                    .marginal_cost
            } else {
                ctx.select::<DeployedScriptInfo<{ DegenQuadraticPoolV1T2T as u8 }>>()
                    .marginal_cost
            };
            match pool_ver {
                QuadraticPoolVer::V1 => {
                    let conf = QuadraticPoolConfig::try_from_pd(pd.clone())?;
                    let reserves_x: TaggedAmount<Rx> =
                        TaggedAmount::new(value.amount_of(conf.asset_x.into()).unwrap_or(0));
                    let executable = conf.operator_pkh == ctx.select::<OperatorCred>().into();
                    let reserves_in_bounds = reserves_x.untag() < conf.x_cap_thr;
                    let pool_id = PoolId::try_from(conf.pool_nft).ok()?;
                    let virgin = ctx
                        .select::<Option<Mints>>()
                        .is_some_and(|mnt| mnt.contains_mint(pool_id.into()));
                    if executable && reserves_in_bounds {
                        return Some(QuadraticPool {
                            id: pool_id,
                            reserves_x,
                            reserves_y: TaggedAmount::new(value.amount_of(conf.asset_y.into())?),
                            asset_x: conf.asset_x,
                            asset_y: conf.asset_y,
                            a_num: conf.a_num,
                            b_num: conf.b_num,
                            ver: pool_ver,
                            accumulated_x_fee: TaggedAmount::new(0),
                            x_fee_num: 0,
                            marginal_cost,
                            bounds,
                            cap_thr: conf.x_cap_thr,
                            virgin,
                            capped: is_capped(ctx),
                        });
                    } else {
                        trace!(
                            "QuadraticPool: {}, executable: {}, reserves_in_bounds: {}, version: {:?}",
                            conf.pool_nft.untag(),
                            executable,
                            reserves_in_bounds,
                            pool_ver
                        );
                    }
                }
                QuadraticPoolVer::V1T2T => {
                    let conf = QuadraticPoolT2TConfig::try_from_pd(pd.clone())?;
                    let reserves_x: TaggedAmount<Rx> =
                        TaggedAmount::new(value.amount_of(conf.asset_x.into()).unwrap_or(0));
                    let executable = conf.operator_pkh == ctx.select::<OperatorCred>().into();
                    let reserves_in_bounds = (reserves_x.untag() - conf.accumulated_x_fee) < conf.x_cap_thr;
                    let pool_id = PoolId::try_from(conf.pool_nft).ok()?;
                    let virgin = ctx
                        .select::<Option<Mints>>()
                        .is_some_and(|mnt| mnt.contains_mint(pool_id.into()));
                    if executable && reserves_in_bounds {
                        return Some(QuadraticPool {
                            id: pool_id,
                            reserves_x,
                            reserves_y: TaggedAmount::new(value.amount_of(conf.asset_y.into())?),
                            asset_x: conf.asset_x,
                            asset_y: conf.asset_y,
                            a_num: conf.a_num,
                            b_num: conf.b_num,
                            ver: pool_ver,
                            accumulated_x_fee: TaggedAmount::new(conf.accumulated_x_fee),
                            x_fee_num: T2T_FEE_NUM,
                            marginal_cost,
                            bounds,
                            cap_thr: conf.x_cap_thr,
                            virgin,
                            capped: is_capped(ctx),
                        });
                    } else {
                        trace!(
                            "QuadraticPool: {}, executable: {}, reserves_in_bounds: {}, version: {:?}",
                            conf.pool_nft.untag(),
                            executable,
                            reserves_in_bounds,
                            pool_ver
                        );
                    }
                }
            }
        };
        None
    }
}

pub fn unsafe_update_t2t_pd(data: &mut PlutusData, new_accumulated_x_fee: TaggedAmount<Rx>) {
    let cpd = data.get_constr_pd_mut().unwrap();
    cpd.set_field(
        DATUM_T2T_MAPPING.accumulated_x_fee,
        new_accumulated_x_fee.untag().into_pd(),
    );
}

impl IntoLedger<TransactionOutput, ImmutablePoolUtxo> for QuadraticPool {
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
        let Token(nft_lq, name_nft) = self.id.into();
        ma.set(nft_lq, name_nft.into(), 1);

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
    use cml_chain::auxdata::Metadata;
    use cml_chain::transaction::TransactionOutput;
    use cml_core::serialization::Deserialize;
    use cml_crypto::{Ed25519KeyHash, ScriptHash};
    use rand::prelude::StdRng;
    use rand::seq::SliceRandom;
    use rand::{Rng, SeedableRng};
    use type_equalities::IsEqual;

    use bloom_offchain::execution_engine::liquidity_book::core::Next;
    use bloom_offchain::execution_engine::liquidity_book::market_maker::{
        AvailableLiquidity, MakerBehavior, MarketMaker,
    };
    use bloom_offchain::execution_engine::liquidity_book::side::OnSide;
    use bloom_offchain::execution_engine::liquidity_book::side::OnSide::{Ask, Bid};
    use bloom_offchain::execution_engine::liquidity_book::types::AbsolutePrice;
    use spectrum_cardano_lib::ex_units::ExUnits;
    use spectrum_cardano_lib::{AssetClass, AssetName, TaggedAmount, TaggedAssetClass, Token};
    use spectrum_offchain::domain::Has;
    use spectrum_offchain::ledger::TryFromLedger;

    use crate::creds::OperatorCred;
    use crate::data::pool::PoolValidation;
    use crate::data::quadratic_pool::{QuadraticPool, QuadraticPoolVer, T2T_FEE_NUM};
    use crate::data::PoolId;
    use crate::deployment::ProtocolValidator::{DegenQuadraticPoolV1, DegenQuadraticPoolV1T2T};
    use crate::deployment::{DeployedScriptInfo, DeployedValidators, ProtocolScriptHashes};
    use crate::handler_context::Mints;
    use crate::pool_math::degen_quadratic_math::{calculate_a_num, A_DENOM, MIN_ADA, TOKEN_EMISSION};

    const LOVELACE: u64 = 1_000_000;
    const DEC: u64 = 1_000;

    fn gen_ada_token_pool(
        reserves_x: u64,
        reserves_y: u64,
        a_num: u64,
        b_num: u64,
        ada_thr: u64,
        reversed: bool,
    ) -> QuadraticPool {
        QuadraticPool {
            id: PoolId::from(Token(
                ScriptHash::from([
                    162, 206, 112, 95, 150, 240, 52, 167, 61, 102, 158, 92, 11, 47, 25, 41, 48, 224, 188,
                    211, 138, 203, 27, 107, 246, 89, 115, 157,
                ]),
                AssetName::from((
                    3,
                    [
                        110, 102, 116, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
                        0, 0, 0, 0, 0, 0,
                    ],
                )),
            )),
            reserves_x: TaggedAmount::new(reserves_x + MIN_ADA),
            reserves_y: TaggedAmount::new(reserves_y),
            asset_x: if !reversed {
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
            asset_y: if !reversed {
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
            } else {
                TaggedAssetClass::new(AssetClass::Native)
            },
            a_num,
            b_num,
            ver: QuadraticPoolVer::V1,
            accumulated_x_fee: TaggedAmount::new(0),
            x_fee_num: 0,
            marginal_cost: ExUnits { mem: 100, steps: 100 },
            bounds: PoolValidation {
                min_n2t_lovelace: 10000000,
                min_t2t_lovelace: 10000000,
            },
            cap_thr: ada_thr + MIN_ADA,
            virgin: false,
            capped: false,
        }
    }

    fn gen_t2t_pool(
        reserves_x: u64,
        reserves_y: u64,
        a_num: u64,
        b_num: u64,
        accumulated_x_fee: u64,
        token_cap_thr: u64,
    ) -> QuadraticPool {
        QuadraticPool {
            id: PoolId::from(Token(
                ScriptHash::from([
                    162, 206, 112, 95, 150, 240, 52, 167, 61, 102, 158, 92, 11, 47, 25, 41, 48, 224, 188,
                    211, 138, 203, 27, 107, 246, 89, 115, 157,
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
            asset_x: TaggedAssetClass::new(AssetClass::Token(Token(
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
            asset_y: TaggedAssetClass::new(AssetClass::Token(Token(
                ScriptHash::from([
                    75, 52, 89, 253, 24, 161, 219, 171, 226, 7, 205, 25, 201, 149, 26, 159, 172, 159, 92, 15,
                    156, 56, 78, 61, 151, 239, 186, 38,
                ]),
                AssetName::from((
                    9,
                    [
                        116, 101, 115, 116, 67, 1, 2, 3, 4, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
                        0, 0, 0, 0, 0, 0, 0,
                    ],
                )),
            ))),
            a_num,
            b_num,
            ver: QuadraticPoolVer::V1T2T,
            accumulated_x_fee: TaggedAmount::new(accumulated_x_fee),
            x_fee_num: T2T_FEE_NUM,
            marginal_cost: ExUnits { mem: 100, steps: 100 },
            bounds: PoolValidation {
                min_n2t_lovelace: 10000000,
                min_t2t_lovelace: 10000000,
            },
            cap_thr: token_cap_thr,
            virgin: false,
            capped: false,
        }
    }

    #[test]
    fn deposit_redeem_consistency_test() {
        let mut rng = StdRng::seed_from_u64(42);
        for _ in 0..1_000 {
            let ada_cap: u64 = rng.gen_range(1_000..100_000) * LOVELACE;
            let a: u64 = calculate_a_num(&ada_cap);
            let to_dep_init: u64 = rng.gen_range(100_000..1_000_000_000);
            let to_dep: u64 = rng.gen_range(100_000..1_000_000_000);
            let b: u64 = 0;

            let pool = gen_ada_token_pool(0, TOKEN_EMISSION, a, b, ada_cap, false);

            let Next::Succ(pool0) = pool.swap(OnSide::Ask(to_dep_init)) else {
                panic!()
            };

            let Next::Succ(pool1) = pool0.swap(OnSide::Ask(to_dep)) else {
                panic!()
            };

            let y_rec = pool0.reserves_y.untag() - pool1.reserves_y.untag();

            let Next::Succ(pool2) = pool1.swap(OnSide::Bid(y_rec)) else {
                panic!()
            };

            let x_rec = pool1.reserves_x.untag() - pool2.reserves_x.untag();
            assert!((to_dep - x_rec) * DEC <= to_dep);
        }
        for _ in 0..1_000 {
            let ada_cap: u64 = rng.gen_range(1_000..100_000) * LOVELACE;
            let a: u64 = calculate_a_num(&ada_cap);
            let to_dep_init: u64 = rng.gen_range(100_000..1_000_000_000);
            let to_dep: u64 = rng.gen_range(100_000..1_000_000_000);
            let b: u64 = 0;

            let pool = gen_ada_token_pool(0, TOKEN_EMISSION, a, b, ada_cap, true);

            let Next::Succ(pool0) = pool.swap(OnSide::Bid(to_dep_init)) else {
                panic!()
            };

            let Next::Succ(pool1) = pool0.swap(OnSide::Bid(to_dep)) else {
                panic!()
            };

            let y_rec = pool0.reserves_y.untag() - pool1.reserves_y.untag();

            let Next::Succ(pool2) = pool1.swap(OnSide::Ask(y_rec)) else {
                panic!()
            };

            let x_rec = pool1.reserves_x.untag() - pool2.reserves_x.untag();
            assert!((to_dep - x_rec) * DEC <= to_dep);
        }
    }

    #[test]
    fn full_flow_test() {
        let mut rng = StdRng::seed_from_u64(42);
        let min_redeem = 10u64;
        let b: u64 = 0;
        for _ in 0..10 {
            let ada_cap: u64 = rng.gen_range(1_000..100_000 * LOVELACE);
            let a: u64 = calculate_a_num(&ada_cap);
            let to_dep_init: u64 = rng.gen_range(LOVELACE..100 * LOVELACE);

            let pool = gen_ada_token_pool(0, TOKEN_EMISSION, a, b, ada_cap, false);

            let Next::Succ(pool) = pool.swap(OnSide::Ask(to_dep_init)) else {
                panic!()
            };
            let mut total_dep = pool.reserves_x.untag().clone();
            let mut reserves_y = pool.reserves_y.untag().clone();
            let mut last_price_num = 0;
            let mut n_operations = 0;
            while total_dep < ada_cap {
                let side = [0, 1, 0].choose(&mut rand::thread_rng()).unwrap();

                let pool_next = if *side == 0 {
                    let cap_remaining = ada_cap - pool.reserves_x.untag();
                    let in_amount: u64 = rng.gen_range(LOVELACE..ada_cap);
                    let in_amount = if in_amount < cap_remaining {
                        in_amount
                    } else {
                        cap_remaining
                    };
                    pool.swap(OnSide::Ask(in_amount))
                } else {
                    let total_supp = TOKEN_EMISSION - reserves_y;
                    let out_amount: u64 = rng.gen_range(min_redeem..total_supp / DEC);
                    let out_amount = if out_amount < total_supp {
                        out_amount
                    } else {
                        continue;
                    };
                    pool.swap(OnSide::Bid(out_amount))
                };
                let Next::Succ(pool) = pool_next else { panic!() };
                total_dep = pool.reserves_x.untag().clone();
                reserves_y = pool.reserves_y.untag().clone();
                last_price_num = pool.a_num as u128
                    * (TOKEN_EMISSION - reserves_y) as u128
                    * (TOKEN_EMISSION - reserves_y) as u128;
                n_operations += 1
            }
            assert_eq!(ada_cap, total_dep);
            assert!(last_price_num <= ada_cap as u128 * A_DENOM / reserves_y as u128)
        }
        for _ in 0..10 {
            let ada_cap: u64 = rng.gen_range(1_000..100_000 * LOVELACE);
            let a: u64 = calculate_a_num(&ada_cap);
            let to_dep_init: u64 = rng.gen_range(LOVELACE..100 * LOVELACE);

            let pool = gen_ada_token_pool(0, TOKEN_EMISSION, a, b, ada_cap, true);

            let Next::Succ(pool) = pool.swap(OnSide::Bid(to_dep_init)) else {
                panic!()
            };
            let mut total_dep = pool.reserves_x.untag().clone();
            let mut reserves_y = pool.reserves_y.untag().clone();
            let mut last_price_num = 0;
            let mut n_operations = 0;
            while total_dep < ada_cap {
                let side = [0, 1, 0].choose(&mut rand::thread_rng()).unwrap();

                let pool_next = if *side == 0 {
                    let cap_remaining = ada_cap - pool.reserves_x.untag();
                    let in_amount: u64 = rng.gen_range(LOVELACE..ada_cap);
                    let in_amount = if in_amount < cap_remaining {
                        in_amount
                    } else {
                        cap_remaining
                    };
                    pool.swap(OnSide::Bid(in_amount))
                } else {
                    let total_supp = TOKEN_EMISSION - reserves_y;
                    let out_amount: u64 = rng.gen_range(min_redeem..total_supp / DEC);
                    let out_amount = if out_amount < total_supp {
                        out_amount
                    } else {
                        continue;
                    };
                    pool.swap(OnSide::Ask(out_amount))
                };
                let Next::Succ(pool) = pool_next else { panic!() };
                total_dep = pool.reserves_x.untag().clone();
                reserves_y = pool.reserves_y.untag().clone();
                last_price_num = pool.a_num as u128
                    * (TOKEN_EMISSION - reserves_y) as u128
                    * (TOKEN_EMISSION - reserves_y) as u128;
                n_operations += 1
            }
            assert_eq!(ada_cap, total_dep);
            assert!(last_price_num <= ada_cap as u128 * A_DENOM / reserves_y as u128)
        }
    }

    #[test]
    fn test_deposit_redeem_fixtures() {
        let min_utxo = 1_000;
        let ada_cap: u64 = 42_014 * LOVELACE;
        let a: u64 = calculate_a_num(&ada_cap);
        let b: u64 = 0;
        let to_dep: u64 = 123_010_079;
        let pool0 = gen_ada_token_pool(min_utxo, TOKEN_EMISSION, a, b, ada_cap, false);

        let Next::Succ(pool1) = pool0.swap(OnSide::Ask(to_dep)) else {
            panic!()
        };

        let y_rec = pool0.reserves_y.untag() - pool1.reserves_y.untag();

        let Next::Succ(pool2) = pool1.swap(OnSide::Bid(y_rec)) else {
            panic!()
        };

        let x_rec = pool1.reserves_x.untag() - pool2.reserves_x.untag();
        let y_real_delta = pool2.reserves_y.untag() - pool1.reserves_y.untag();

        assert_eq!(a, 298759111111);
        assert_eq!(y_rec, 107295192);
        assert_eq!(x_rec, 123010076);
        assert_eq!(y_real_delta, y_rec);
    }

    #[test]
    fn test_available_lq() {
        let ada_cap: u64 = 25_240 * LOVELACE;
        let a: u64 = 174_150_190_999;
        let b: u64 = 2_000_000;
        let to_dep: u64 = 123_010_079;
        let pool0 = gen_ada_token_pool(0, TOKEN_EMISSION, a, b, ada_cap, false);

        let Next::Succ(pool1) = pool0.swap(Ask(to_dep)) else {
            panic!()
        };
        let y_rec = pool0.reserves_y.untag() - pool1.reserves_y.untag();

        let worst_price = AbsolutePrice::new(y_rec, to_dep).unwrap();
        let Some(AvailableLiquidity { input: b, output: q }) =
            pool0.available_liquidity_on_side(Ask(worst_price))
        else {
            panic!()
        };
        assert_eq!(q, 56319924);
        assert_eq!(b, 123010090);

        let Next::Succ(pool2) = pool1.swap(Bid(y_rec / 2)) else {
            panic!()
        };

        let x_rec = pool1.reserves_x.untag() - pool2.reserves_x.untag();

        let w_price = AbsolutePrice::new(x_rec, y_rec / 2).unwrap();

        let Some(AvailableLiquidity { input: b, output: q }) =
            pool1.available_liquidity_on_side(Bid(w_price))
        else {
            panic!()
        };
        assert_eq!(q, 65393877);
        assert_eq!(b, 28159959);

        let too_high_ask_price = AbsolutePrice::new(1, 300).unwrap();

        let Some(AvailableLiquidity { input: b, output: q }) =
            pool1.available_liquidity_on_side(Ask(too_high_ask_price))
        else {
            panic!()
        };
        let Next::Succ(pool3) = pool1.swap(Ask(pool1.cap_thr - pool1.reserves_x.untag())) else {
            panic!()
        };
        let y_max = pool1.reserves_y.untag() - pool3.reserves_y.untag();
        assert_eq!(q, y_max);
        assert_eq!(b, pool1.cap_thr - pool1.reserves_x.untag());
    }
    #[test]
    fn test_available_lq_reversed() {
        let ada_cap: u64 = 25_240 * LOVELACE;
        let a: u64 = 174_150_190_999;
        let b: u64 = 2_000_000;
        let to_dep: u64 = 123_010_079;
        let pool0 = gen_ada_token_pool(0, TOKEN_EMISSION, a, b, ada_cap, true);

        let Next::Succ(pool1) = pool0.swap(Bid(to_dep)) else {
            panic!()
        };
        let y_rec = pool0.reserves_y.untag() - pool1.reserves_y.untag();

        let worst_price = AbsolutePrice::new(to_dep, y_rec).unwrap();
        let Some(AvailableLiquidity { input: b, output: q }) =
            pool0.available_liquidity_on_side(Bid(worst_price))
        else {
            panic!()
        };
        assert_eq!(q, 56319924);
        assert_eq!(b, 123010090);

        let Next::Succ(pool2) = pool1.swap(Ask(y_rec / 2)) else {
            panic!()
        };

        let x_rec = pool1.reserves_x.untag() - pool2.reserves_x.untag();

        let w_price = AbsolutePrice::new(y_rec / 2, x_rec).unwrap();

        let Some(AvailableLiquidity { input: b, output: q }) =
            pool1.available_liquidity_on_side(Ask(w_price))
        else {
            panic!()
        };
        assert_eq!(q, 65393877);
        assert_eq!(b, 28159959);

        let too_high_ask_price = AbsolutePrice::new(300, 1).unwrap();

        let Some(AvailableLiquidity { input: b, output: q }) =
            pool1.available_liquidity_on_side(Bid(too_high_ask_price))
        else {
            panic!()
        };
        let Next::Succ(pool3) = pool1.swap(Bid(pool1.cap_thr - pool1.reserves_x.untag())) else {
            panic!()
        };
        let y_max = pool1.reserves_y.untag() - pool3.reserves_y.untag();
        assert_eq!(q, y_max);
        assert_eq!(b, pool1.cap_thr - pool1.reserves_x.untag());
    }

    #[test]
    fn test_available_lq_gen() {
        let mut rng = StdRng::seed_from_u64(42);
        let ada_cap: u64 = 25_240 * LOVELACE;
        let a: u64 = 174_150_190_999;
        let b: u64 = 2_000_000;
        let mut pool = gen_ada_token_pool(0, TOKEN_EMISSION, a, b, ada_cap, false);

        for _ in 0..10 {
            let num = rng.gen_range(0..ada_cap);
            let denom = rng.gen_range(0..TOKEN_EMISSION);

            let worst_price = AbsolutePrice::new(num, denom).unwrap();
            let _ = pool.available_liquidity_on_side(Ask(worst_price));
            let _ = pool.available_liquidity_on_side(Bid(worst_price));
            let Next::Succ(pool1) = pool.swap(Ask(num / 100)) else {
                panic!()
            };
            pool = pool1;
        }

        let mut pool = gen_ada_token_pool(0, TOKEN_EMISSION, a, b, ada_cap, true);
        for _ in 0..10 {
            let num = rng.gen_range(0..ada_cap);
            let denom = rng.gen_range(0..TOKEN_EMISSION);

            let worst_price = AbsolutePrice::new(denom, num).unwrap();
            let _ = pool.available_liquidity_on_side(Bid(worst_price));
            let _ = pool.available_liquidity_on_side(Ask(worst_price));
            let Next::Succ(pool1) = pool.swap(Bid(num / 100)) else {
                panic!()
            };
            pool = pool1;
        }
    }

    #[test]
    fn test_available_lq_drain() {
        let ada_cap: u64 = 25_240 * LOVELACE;
        let a: u64 = 174_150_190_999;
        let b: u64 = 2_000_000;
        let pool0 = gen_ada_token_pool(100 * LOVELACE, 953011295, a, b, ada_cap, false);

        let Next::Succ(pool1) = pool0.swap(Bid(46988705)) else {
            panic!()
        };
        let x_rec = pool0.reserves_x.untag() - pool1.reserves_x.untag();

        let worst_price = AbsolutePrice::new(x_rec / 2, 46988705).unwrap();

        let Some(AvailableLiquidity { input: b, output: q }) =
            pool0.available_liquidity_on_side(Bid(worst_price))
        else {
            panic!()
        };
        assert_eq!(q, 100000000);
        assert_eq!(b, 46988705);

        let Next::Succ(pool1) = pool0.swap(Bid(b)) else {
            panic!()
        };

        let rec = pool0.reserves_x.untag() - pool1.reserves_x.untag();
        assert!((q - rec) * DEC <= q);

        let mut pool = gen_ada_token_pool(0, TOKEN_EMISSION, a, b, ada_cap, false);

        let mut rng = StdRng::seed_from_u64(42);
        for _ in 0..10 {
            let to_dep = rng.gen_range(0..ada_cap);
            let Next::Succ(pool1) = pool.swap(Ask(to_dep)) else {
                panic!()
            };
            let y_rec = pool.reserves_y.untag() - pool1.reserves_y.untag();

            let Next::Succ(pool2) = pool1.swap(Bid(y_rec)) else {
                panic!()
            };

            let x_rec = pool1.reserves_x.untag() - pool2.reserves_x.untag();
            assert!((to_dep - x_rec) * DEC <= to_dep);

            let worst_price = AbsolutePrice::new(x_rec / 2, y_rec).unwrap();

            let Some(AvailableLiquidity { input: _, output: q }) =
                pool1.available_liquidity_on_side(Bid(worst_price))
            else {
                panic!()
            };
            assert!((q - x_rec) * DEC <= q);

            let Some(AvailableLiquidity { input: b, output: q }) =
                pool2.available_liquidity_on_side(Bid(worst_price))
            else {
                panic!()
            };
            assert_eq!(b, 0);
            assert_eq!(q, 0);

            let Some(AvailableLiquidity { input: b, output: q }) = pool2
                .available_liquidity_on_side(Ask(AbsolutePrice::new(ada_cap / 2, TOKEN_EMISSION).unwrap()))
            else {
                panic!()
            };
            let Next::Succ(pool3) = pool2.swap(Ask(pool2.cap_thr - pool2.reserves_x.untag())) else {
                panic!()
            };
            let y_max = pool2.reserves_y.untag() - pool3.reserves_y.untag();
            assert_eq!(b, pool2.cap_thr - pool2.reserves_x.untag());
            assert_eq!(q, y_max);
            assert_eq!(pool3.reserves_x.untag(), pool3.cap_thr)
        }
    }

    #[test]
    fn test_available_lq_bug() {
        let ada_cap: u64 = 25_240 * LOVELACE;
        let a: u64 = 174_150_190_999;
        let b: u64 = 2_000_000;
        let pool0 = gen_ada_token_pool(150000000, 933525758, a, b, ada_cap, false);

        let worst_price = AbsolutePrice::new(66474242, 904253).unwrap();
        let Some(AvailableLiquidity { input: b, output: q }) =
            pool0.available_liquidity_on_side(Bid(worst_price))
        else {
            panic!()
        };
        assert_eq!(b, 66474242);
        assert_eq!(q, 150000000);
    }

    #[test]
    fn test_output_estimation() {
        let ada_cap: u64 = 25_240 * LOVELACE;
        let a: u64 = 174_150_190_999;
        let b: u64 = 2_000_000;
        let to_dep: u64 = 123_010_079;
        let pool0 = gen_ada_token_pool(0, TOKEN_EMISSION, a, b, ada_cap, false);

        let Next::Succ(pool1) = pool0.swap(Ask(to_dep)) else {
            panic!()
        };
        let y_rec = pool0.reserves_y.untag() - pool1.reserves_y.untag();

        let Some(AvailableLiquidity { input: b, output: q }) = pool0.estimated_trade(Ask(to_dep)) else {
            panic!()
        };
        assert_eq!(q, y_rec);
        assert_eq!(b, to_dep);

        let Next::Succ(pool2) = pool1.swap(Bid(y_rec)) else {
            panic!()
        };

        let x_rec = pool1.reserves_x.untag() - pool2.reserves_x.untag();

        let Some(AvailableLiquidity { input: b, output: q }) = pool1.estimated_trade(Bid(y_rec)) else {
            panic!()
        };
        assert_eq!(q, x_rec);
        assert_eq!(b, y_rec);

        let too_bid_ask_input = 2 * ada_cap.clone();
        let max_ask_input = (pool2.cap_thr) * 1005 / 1000 - pool2.reserves_x.untag();
        let Some(AvailableLiquidity { input: b, output: q }) = pool2.estimated_trade(Ask(too_bid_ask_input))
        else {
            panic!()
        };
        let Next::Succ(pool3) = pool2.swap(Ask(max_ask_input)) else {
            panic!()
        };
        let y_max = pool2.reserves_y.untag() - pool3.reserves_y.untag();
        assert_eq!(q, y_max);
        assert_eq!(b, max_ask_input);
        assert!(pool3.reserves_x.untag() >= pool3.cap_thr)
    }

    #[test]
    fn test_output_estimation_t2t() {
        let token_cap: u64 = 1_000_000_000;
        let a: u64 = 133079166694;
        let b: u64 = 3750000;
        let to_dep: u64 = 135768687;
        let pool0 = gen_t2t_pool(0, TOKEN_EMISSION, a, b, 0, token_cap);

        let Next::Succ(pool1) = pool0.swap(Ask(to_dep)) else {
            panic!()
        };

        // 964331803 без вычета
        // 964678358 с вычетом

        println!("{:?}", pool1);

        assert_eq!(1, 2)
        // let y_rec = pool0.reserves_y.untag() - pool1.reserves_y.untag();
        //
        // let Some(AvailableLiquidity { input: b, output: q }) = pool0.estimated_trade(Ask(to_dep)) else {
        //     panic!()
        // };
        // assert_eq!(q, y_rec);
        // assert_eq!(b, to_dep);
        //
        // let Next::Succ(pool2) = pool1.swap(Bid(y_rec)) else {
        //     panic!()
        // };
        //
        // let x_rec = pool1.reserves_x.untag() - pool2.reserves_x.untag();
        //
        // let Some(AvailableLiquidity { input: b, output: q }) = pool1.estimated_trade(Bid(y_rec)) else {
        //     panic!()
        // };
        // assert_eq!(q, x_rec);
        // assert_eq!(b, y_rec);
        //
        // let too_bid_ask_input = 2 * ada_cap.clone();
        // let max_ask_input = (pool2.cap_thr) * 1005 / 1000 - pool2.reserves_x.untag();
        // let Some(AvailableLiquidity { input: b, output: q }) = pool2.estimated_trade(Ask(too_bid_ask_input))
        // else {
        //     panic!()
        // };
        // let Next::Succ(pool3) = pool2.swap(Ask(max_ask_input)) else {
        //     panic!()
        // };
        // let y_max = pool2.reserves_y.untag() - pool3.reserves_y.untag();
        // assert_eq!(q, y_max);
        // assert_eq!(b, max_ask_input);
        // assert!(pool3.reserves_x.untag() >= pool3.cap_thr)
    }

    struct Ctx {
        bounds: PoolValidation,
        scripts: ProtocolScriptHashes,
        operator_cred: OperatorCred,
    }

    impl Has<PoolValidation> for Ctx {
        fn select<U: IsEqual<PoolValidation>>(&self) -> PoolValidation {
            self.bounds
        }
    }

    impl Has<OperatorCred> for Ctx {
        fn select<U: IsEqual<OperatorCred>>(&self) -> OperatorCred {
            self.operator_cred
        }
    }

    impl Has<DeployedScriptInfo<{ DegenQuadraticPoolV1 as u8 }>> for Ctx {
        fn select<U: IsEqual<DeployedScriptInfo<{ DegenQuadraticPoolV1 as u8 }>>>(
            &self,
        ) -> DeployedScriptInfo<{ DegenQuadraticPoolV1 as u8 }> {
            DeployedScriptInfo {
                script_hash: ScriptHash::from_hex("905ab869961b094f1b8197278cfe15b45cbe49fa8f32c6b014f85a2d")
                    .unwrap(),
                marginal_cost: ExUnits { mem: 0, steps: 0 },
            }
        }
    }

    impl Has<DeployedScriptInfo<{ DegenQuadraticPoolV1T2T as u8 }>> for Ctx {
        fn select<U: IsEqual<DeployedScriptInfo<{ DegenQuadraticPoolV1T2T as u8 }>>>(
            &self,
        ) -> DeployedScriptInfo<{ DegenQuadraticPoolV1T2T as u8 }> {
            DeployedScriptInfo {
                script_hash: ScriptHash::from_hex("05fca42e405386300c71cb3d3ab80ed65e2838f20073409c0cca0631")
                    .unwrap(),
                marginal_cost: ExUnits { mem: 0, steps: 0 },
            }
        }
    }

    impl Has<Option<Mints>> for Ctx {
        fn select<U: IsEqual<Option<Mints>>>(&self) -> Option<Mints> {
            None
        }
    }

    impl Has<Option<Metadata>> for Ctx {
        fn select<U: IsEqual<Option<Metadata>>>(&self) -> Option<Metadata> {
            None
        }
    }

    #[test]
    fn try_read_pool() {
        let raw_deployment = std::fs::read_to_string("/Users/oskin/dev/spectrum/spectrum-offchain-multiplatform/bloom-cardano-agent/resources/mainnet.deployment.json").expect("Cannot load deployment file");
        let deployment: DeployedValidators =
            serde_json::from_str(&raw_deployment).expect("Invalid deployment file");
        let scripts = ProtocolScriptHashes::from(&deployment);
        let ctx = Ctx {
            scripts,
            operator_cred: OperatorCred(
                Ed25519KeyHash::from_hex("e8d7a0d650a2dc2f6dde52f056d171f4cbcc0719951c42e2df9892a8").unwrap(),
            ),
            bounds: PoolValidation {
                min_n2t_lovelace: 150_000_000,
                min_t2t_lovelace: 10_000_000,
            },
        };
        let bearer = TransactionOutput::from_cbor_bytes(&*hex::decode(POOL_UTXO).unwrap()).unwrap();
        let pool = QuadraticPool::try_from_ledger(&bearer, &ctx);
        println!("Pool: {}", pool.unwrap());
    }

    const POOL_UTXO: &str = "a300581d7005fca42e405386300c71cb3d3ab80ed65e2838f20073409c0cca063101821a05f5e100a2581c1954722030c9adf89d037ebe00bc70747eb746956a8b02f755f789a9a145746f6b656e1a3b9aca00581c9d8f27a66cfffebe2a4a19157b6845a051dd2f627f11bfafed584d51a1436e667401028201d81858f3d8799fd8799f581c9d8f27a66cfffebe2a4a19157b6845a051dd2f627f11bfafed584d51436e6674ffd8799f581cf357c6f00f0496fcd01851a7a8d909a1d9d1c9d7ba9bc021ac3bc3fe4d636e74546f6b656e746f6b656effd8799f581c1954722030c9adf89d037ebe00bc70747eb746956a8b02f755f789a945746f6b656eff1b0000001efc22eee61a00393870581c15772e8f1fdcf12d59636caf42522b7d6249ccb223253eb7e9b6d5091b00000004af5c9bf9581ce67c2ed0ccbea65650a054400a22357a357f581a0b535fc06097278b581c65e55e46a039c5711fcdc508c79ef626b0b4e7be0e6fb3c4548939c0ff";

    #[test]
    fn test_available_lq_bug_low_ask() {
        let ada_cap: u64 = 10_988_175_000;
        let a: u64 = 74_737_329_614;
        let b: u64 = 1_274_818;

        let pool0 = gen_ada_token_pool(6182173147, 1_000_000_000 - 601281779, a, b, ada_cap, false);
        let Next::Succ(pool1) = pool0.swap(Ask(9900000)) else {
            panic!()
        };
        let y_rec = pool0.reserves_y.untag() - pool1.reserves_y.untag();
        assert_eq!(y_rec, 349686);
        let worst_price = AbsolutePrice::new(174843, 10000000).unwrap();
        let Some(AvailableLiquidity { input: b, output: q }) =
            pool0.available_liquidity_on_side(Ask(worst_price))
        else {
            panic!()
        };
        assert_eq!(b, 4806001853);
        assert_eq!(q, 137524195);
    }
}
