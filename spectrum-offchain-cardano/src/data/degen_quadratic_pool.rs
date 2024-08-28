use std::fmt::{Debug, Display, Formatter};
use std::ops::Div;

use bignumber::BigNumber;
use cml_chain::address::Address;
use cml_chain::assets::MultiAsset;
use cml_chain::certs::StakeCredential;
use cml_chain::plutus::{ConstrPlutusData, PlutusData};
use cml_chain::transaction::{ConwayFormatTxOut, TransactionOutput};
use cml_chain::utils::BigInteger;
use cml_chain::Value;
use cml_multi_era::babbage::BabbageTransactionOutput;
use dashu_float::DBig;
use num_traits::{CheckedDiv, CheckedSub, ToPrimitive};
use type_equalities::IsEqual;
use void::Void;

use bloom_offchain::execution_engine::liquidity_book::core::Next;
use bloom_offchain::execution_engine::liquidity_book::market_maker::{
    AbsoluteReserves, AvailableLiquidity, MakerBehavior, MarketMaker, PoolQuality, SpotPrice,
};
use bloom_offchain::execution_engine::liquidity_book::side::{OnSide, Side};
use bloom_offchain::execution_engine::liquidity_book::types::AbsolutePrice;
use spectrum_cardano_lib::ex_units::ExUnits;
use spectrum_cardano_lib::plutus_data::{
    ConstrPlutusDataExtension, DatumExtension, IntoPlutusData, PlutusDataExtension,
};
use spectrum_cardano_lib::transaction::TransactionOutputExtension;
use spectrum_cardano_lib::types::TryFromPData;
use spectrum_cardano_lib::value::ValueExtension;
use spectrum_cardano_lib::{TaggedAmount, TaggedAssetClass, Token};
use spectrum_offchain::data::{Has, Stable, Tradable};
use spectrum_offchain::ledger::{IntoLedger, TryFromLedger};

use crate::data::order::{Base, PoolNft, Quote};
use crate::data::pair::{order_canonical, PairId};
use crate::data::pool::{
    ApplyOrder, CFMMPoolAction, ImmutablePoolUtxo, PoolAssetMapping, PoolValidation, Rx, Ry,
};
use crate::data::PoolId;
use crate::deployment::ProtocolValidator::DegenQuadraticPoolV1;
use crate::deployment::{DeployedScriptInfo, DeployedValidator, DeployedValidatorErased, RequiresValidator};
use crate::fees::FeeExtension;
use crate::pool_math::degen_quadratic_math::{
    degen_quadratic_output_amount, A_DENOM, B_DENOM, FULL_PERCENTILE, MAX_ALLOWED_ADA_EXTRAS_PERCENTILE,
    MIN_ADA, TOKEN_EMISSION,
};

pub struct DegenQuadraticPoolConfig {
    pub pool_nft: TaggedAssetClass<PoolNft>,
    pub asset_x: TaggedAssetClass<Rx>,
    pub asset_y: TaggedAssetClass<Ry>,
    pub a_num: u64,
    pub b_num: u64,
    pub ada_cap_thr: u64,
}

struct DatumMapping {
    pub pool_nft: usize,
    pub asset_x: usize,
    pub asset_y: usize,
    pub a_num: usize,
    pub b_num: usize,
    pub ada_cap_thr: usize,
}

const DATUM_MAPPING: DatumMapping = DatumMapping {
    pool_nft: 0,
    asset_x: 1,
    asset_y: 2,
    a_num: 3,
    b_num: 4,
    ada_cap_thr: 6,
};

impl TryFromPData for DegenQuadraticPoolConfig {
    fn try_from_pd(data: PlutusData) -> Option<Self> {
        let mut cpd = data.into_constr_pd()?;
        let pool_nft = TaggedAssetClass::try_from_pd(cpd.take_field(DATUM_MAPPING.pool_nft)?)?;
        let asset_x = TaggedAssetClass::try_from_pd(cpd.take_field(DATUM_MAPPING.asset_x)?)?;
        let asset_y = TaggedAssetClass::try_from_pd(cpd.take_field(DATUM_MAPPING.asset_y)?)?;
        let a_num = cpd.take_field(DATUM_MAPPING.a_num)?.into_u64()?;
        let b_num = cpd.take_field(DATUM_MAPPING.b_num)?.into_u64()?;
        let ada_cap_thr = cpd.take_field(DATUM_MAPPING.ada_cap_thr)?.into_u64()?;

        Some(Self {
            pool_nft,
            asset_x,
            asset_y,
            a_num,
            b_num,
            ada_cap_thr,
        })
    }
}

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub enum DegenQuadraticPoolVer {
    V1,
}

impl DegenQuadraticPoolVer {
    pub fn try_from_address<Ctx>(pool_addr: &Address, ctx: &Ctx) -> Option<DegenQuadraticPoolVer>
    where
        Ctx: Has<DeployedScriptInfo<{ DegenQuadraticPoolV1 as u8 }>> + Has<PoolValidation>,
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
                return Some(DegenQuadraticPoolVer::V1);
            }
        }
        None
    }
}

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub struct DegenQuadraticPool {
    pub id: PoolId,
    pub reserves_x: TaggedAmount<Rx>,
    pub reserves_y: TaggedAmount<Ry>,
    pub asset_x: TaggedAssetClass<Rx>,
    pub asset_y: TaggedAssetClass<Ry>,
    pub a_num: u64,
    pub b_num: u64,
    pub ada_cap_thr: u64,
    pub ver: DegenQuadraticPoolVer,
    pub marginal_cost: ExUnits,
    pub bounds: PoolValidation,
}

impl Display for DegenQuadraticPool {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(&*format!(
            "DegenPool(id: {}, static_price: {}, quality: {})",
            self.id,
            self.static_price(),
            self.quality()
        ))
    }
}

impl DegenQuadraticPool {
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

pub struct DegenQuadraticPoolRedeemer {
    pub pool_input_index: u64,
    pub pool_output_index: u64,
    pub action: CFMMPoolAction,
}

impl DegenQuadraticPoolRedeemer {
    pub fn to_plutus_data(self) -> PlutusData {
        DegenQuadraticPool::create_redeemer(self.pool_input_index, self.pool_output_index)
    }
}

pub trait AMMOps {
    fn output_amount(
        &self,
        base_asset: TaggedAssetClass<Base>,
        base_amount: TaggedAmount<Base>,
    ) -> TaggedAmount<Quote>;
}

impl AMMOps for DegenQuadraticPool {
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
        )
    }
}

impl<Ctx> RequiresValidator<Ctx> for DegenQuadraticPool
where
    Ctx: Has<DeployedValidator<{ DegenQuadraticPoolV1 as u8 }>>,
{
    fn get_validator(&self, ctx: &Ctx) -> DeployedValidatorErased {
        match self.ver {
            DegenQuadraticPoolVer::V1 => ctx
                .select::<DeployedValidator<{ DegenQuadraticPoolV1 as u8 }>>()
                .erased(),
        }
    }
}

impl MakerBehavior for DegenQuadraticPool {
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

impl MarketMaker for DegenQuadraticPool {
    type U = ExUnits;

    fn static_price(&self) -> SpotPrice {
        let x = self.asset_x.untag();
        let y = self.asset_y.untag();
        let [base, _] = order_canonical(x, y);
        let price_num =
            (self.reserves_y.untag() as u128 * self.reserves_y.untag() as u128 * self.a_num as u128)
                * B_DENOM
                + A_DENOM * self.b_num as u128;
        let price_denom = A_DENOM * B_DENOM;
        if x == base {
            AbsolutePrice::new_raw(price_num, price_denom).into()
        } else {
            AbsolutePrice::new_raw(price_denom, price_num).into()
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
            OnSide::Ask(input) if self.reserves_x.untag() + input <= self.ada_cap_thr => (
                input,
                self.output_amount(TaggedAssetClass::new(base), TaggedAmount::new(input))
                    .untag(),
            ),
            // Swap will overfill ada_cap_thr with corresponding input. So, we couldn't execute it
            OnSide::Ask(_) => return None,
        };
        AbsolutePrice::new(quote, base)
    }
    fn quality(&self) -> PoolQuality {
        PoolQuality::from((self.reserves_x.untag() - MIN_ADA) as u128)
    }

    fn marginal_cost_hint(&self) -> Self::U {
        self.marginal_cost
    }

    fn is_active(&self) -> bool {
        self.reserves_x.untag()
            < self.ada_cap_thr * (MAX_ALLOWED_ADA_EXTRAS_PERCENTILE + FULL_PERCENTILE) / FULL_PERCENTILE
    }

    fn liquidity(&self) -> AbsoluteReserves {
        let x = self.asset_x.untag();
        let y = self.asset_y.untag();
        let [base, _] = order_canonical(x, y);
        if base == x {
            AbsoluteReserves {
                base: self.reserves_x.untag() - MIN_ADA,
                quote: self.reserves_y.untag(),
            }
        } else {
            AbsoluteReserves {
                base: self.reserves_y.untag(),
                quote: self.reserves_x.untag() - MIN_ADA,
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

        let x0_val = (self.reserves_x.untag() - MIN_ADA) as f64;
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

        let max_ada =
            BigNumber::from(((self.ada_cap_thr - MIN_ADA) * (PERC + MAX_EXCESS_PERC) / PERC) as f64);
        let min_ada = BigNumber::from(MIN_ADA as f64);
        if x0_val >= (self.ada_cap_thr - MIN_ADA) as f64 {
            return None;
        };

        let n_0 = BigNumber::from(0f64);
        let n_1_minus = BigNumber::from(-1f64);
        let n_1 = BigNumber::from(1f64);
        let n_2 = BigNumber::from(2f64);
        let bound_price =
            n_1.clone() / (a.clone() * BigNumber::from(TOKEN_EMISSION as f64).pow(&n_2) + b.clone());

        let mut err = n_2.clone();

        let (output_amount, input_amount) = match worst_price {
            OnSide::Ask(price) => {
                const COEFF_DENOM: u64 = 100000000000000;
                const COEFF_1_NUM: u64 = 26456684199470;
                const COEFF_0_NUM: u64 = 377976314968462;

                let p = BigNumber::from(*price.numer() as f64) / BigNumber::from(*price.denom() as f64);
                if p.value < bound_price.value {
                    let ada_delta = self.ada_cap_thr - x0_val as u64 - MIN_ADA;
                    let token_delta = self
                        .output_amount(
                            TaggedAssetClass::new(self.asset_x.into()),
                            TaggedAmount::new(ada_delta),
                        )
                        .untag();
                    return Some(AvailableLiquidity {
                        input: ada_delta,
                        output: token_delta,
                    });
                };
                let n_81 = BigNumber::from(81f64);

                let coeff_denom = BigNumber::from(COEFF_DENOM as f64);
                let coeff_0 = BigNumber::from(COEFF_0_NUM as f64) / coeff_denom.clone();
                let coeff_1 = BigNumber::from(COEFF_1_NUM as f64) / coeff_denom;

                let mut x1 = BigNumber::from((self.ada_cap_thr - MIN_ADA) as f64);

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
                    if x_new.value.clone() < max_ada.value.clone()
                        && x_new.value.clone() > min_ada.value.clone()
                    {
                        x1 = x1.clone() - additional.clone()
                    } else {
                        let ada_delta = self.ada_cap_thr - x0_val as u64 - MIN_ADA;
                        let token_delta = self
                            .output_amount(
                                TaggedAssetClass::new(self.asset_x.into()),
                                TaggedAmount::new(ada_delta),
                            )
                            .untag();
                        return Some(AvailableLiquidity {
                            input: ada_delta,
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
            OnSide::Bid(price) => {
                let n_3 = BigNumber::from(3f64);
                let n_4 = BigNumber::from(4f64);
                let n_6 = BigNumber::from(6f64);
                let n_18 = BigNumber::from(18f64);
                let n_243 = BigNumber::from(243f64);
                let n_729 = BigNumber::from(729f64);

                let mut counter = 0usize;
                let p = BigNumber::from(*price.denom() as f64) / BigNumber::from(*price.numer() as f64);
                if p.value < bound_price.value {
                    let supply_y0_val = supply_y0.value.to_f64().value() as u64;
                    let x_val = if supply_y0_val > 0u64 { x0_val } else { 0f64 };
                    return Some(AvailableLiquidity {
                        output: x_val as u64,
                        input: supply_y0.value.to_f64().value() as u64,
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
                    if x_new.value.clone() < max_ada.value.clone()
                        && x_new.value.clone() > min_ada.value.clone()
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

    fn output_estimation(&self, input: OnSide<u64>) -> Option<AvailableLiquidity> {
        const MAX_EXCESS_PERC: u64 = 1;
        const PERC: u64 = 100;

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
                let reserves_ada = self.reserves_x.untag();
                let max_ada_required =
                    (self.ada_cap_thr - MIN_ADA) * (PERC + MAX_EXCESS_PERC) / PERC - reserves_ada;
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

impl Has<DegenQuadraticPoolVer> for DegenQuadraticPool {
    fn select<U: IsEqual<DegenQuadraticPoolVer>>(&self) -> DegenQuadraticPoolVer {
        self.ver
    }
}

impl Stable for DegenQuadraticPool {
    type StableId = Token;
    fn stable_id(&self) -> Self::StableId {
        self.id.into()
    }
    fn is_quasi_permanent(&self) -> bool {
        true
    }
}

impl Tradable for DegenQuadraticPool {
    type PairId = PairId;
    fn pair_id(&self) -> Self::PairId {
        PairId::canonical(self.asset_x.untag(), self.asset_y.untag())
    }
}

impl<Ctx> TryFromLedger<BabbageTransactionOutput, Ctx> for DegenQuadraticPool
where
    Ctx: Has<DeployedScriptInfo<{ DegenQuadraticPoolV1 as u8 }>> + Has<PoolValidation>,
{
    fn try_from_ledger(repr: &BabbageTransactionOutput, ctx: &Ctx) -> Option<Self> {
        if let Some(pool_ver) = DegenQuadraticPoolVer::try_from_address(repr.address(), ctx) {
            let value = repr.value();
            let pd = repr.datum().clone()?.into_pd()?;
            let bounds = ctx.select::<PoolValidation>();
            let marginal_cost = match pool_ver {
                DegenQuadraticPoolVer::V1 => {
                    ctx.select::<DeployedScriptInfo<{ DegenQuadraticPoolV1 as u8 }>>()
                        .marginal_cost
                }
            };
            match pool_ver {
                DegenQuadraticPoolVer::V1 => {
                    let conf = DegenQuadraticPoolConfig::try_from_pd(pd.clone())?;
                    let reserves_x: TaggedAmount<Rx> =
                        TaggedAmount::new(value.amount_of(conf.asset_x.into())?);

                    if conf.asset_x.is_native() && reserves_x.untag() < conf.ada_cap_thr {
                        return Some(DegenQuadraticPool {
                            id: PoolId::try_from(conf.pool_nft).ok()?,
                            reserves_x,
                            reserves_y: TaggedAmount::new(value.amount_of(conf.asset_y.into())?),
                            asset_x: conf.asset_x,
                            asset_y: conf.asset_y,
                            a_num: conf.a_num,
                            b_num: conf.b_num,
                            ver: pool_ver,
                            marginal_cost,
                            bounds,
                            ada_cap_thr: conf.ada_cap_thr,
                        });
                    }
                }
            };
        };
        None
    }
}

impl IntoLedger<TransactionOutput, ImmutablePoolUtxo> for DegenQuadraticPool {
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
    use cml_crypto::ScriptHash;
    use rand::prelude::StdRng;
    use rand::seq::SliceRandom;
    use rand::{Rng, SeedableRng};

    use bloom_offchain::execution_engine::liquidity_book::core::Next;
    use bloom_offchain::execution_engine::liquidity_book::market_maker::{
        AvailableLiquidity, MakerBehavior, MarketMaker,
    };
    use bloom_offchain::execution_engine::liquidity_book::side::OnSide;
    use bloom_offchain::execution_engine::liquidity_book::side::OnSide::{Ask, Bid};
    use bloom_offchain::execution_engine::liquidity_book::types::AbsolutePrice;
    use spectrum_cardano_lib::ex_units::ExUnits;
    use spectrum_cardano_lib::{AssetClass, AssetName, TaggedAmount, TaggedAssetClass, Token};

    use crate::data::degen_quadratic_pool::{DegenQuadraticPool, DegenQuadraticPoolVer};
    use crate::data::pool::PoolValidation;
    use crate::data::PoolId;
    use crate::pool_math::degen_quadratic_math::{calculate_a_num, A_DENOM, MIN_ADA, TOKEN_EMISSION};

    const LOVELACE: u64 = 1_000_000;
    const DEC: u64 = 1_000;

    fn gen_ada_token_pool(
        reserves_x: u64,
        reserves_y: u64,
        a_num: u64,
        b_num: u64,
        ada_thr: u64,
    ) -> DegenQuadraticPool {
        return DegenQuadraticPool {
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
            a_num,
            b_num,
            ver: DegenQuadraticPoolVer::V1,
            marginal_cost: ExUnits { mem: 100, steps: 100 },
            bounds: PoolValidation {
                min_n2t_lovelace: 10000000,
                min_t2t_lovelace: 10000000,
            },
            ada_cap_thr: ada_thr + MIN_ADA,
        };
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

            let pool = gen_ada_token_pool(0, TOKEN_EMISSION, a, b, ada_cap);

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

            let pool = gen_ada_token_pool(0, TOKEN_EMISSION, a, b, ada_cap);

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
    }

    #[test]
    fn test_deposit_redeem_fixtures() {
        let min_utxo = 1_000;
        let ada_cap: u64 = 42_014 * LOVELACE;
        let a: u64 = calculate_a_num(&ada_cap);
        let b: u64 = 0;
        let to_dep: u64 = 123_010_079;
        let pool0 = gen_ada_token_pool(min_utxo, TOKEN_EMISSION, a, b, ada_cap);

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
        let pool0 = gen_ada_token_pool(0, TOKEN_EMISSION, a, b, ada_cap);

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
        let Next::Succ(pool3) = pool1.swap(Ask(pool1.ada_cap_thr - pool1.reserves_x.untag())) else {
            panic!()
        };
        let y_max = pool1.reserves_y.untag() - pool3.reserves_y.untag();
        assert_eq!(q, y_max);
        assert_eq!(b, pool1.ada_cap_thr - pool1.reserves_x.untag());
    }

    #[test]
    fn test_available_lq_gen() {
        let mut rng = StdRng::seed_from_u64(42);
        let ada_cap: u64 = 25_240 * LOVELACE;
        let a: u64 = 174_150_190_999;
        let b: u64 = 2_000_000;
        let mut pool = gen_ada_token_pool(3000000, TOKEN_EMISSION, a, b, ada_cap);

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
    }
    #[test]
    fn test_available_lq_drain() {
        let ada_cap: u64 = 25_240 * LOVELACE;
        let a: u64 = 174_150_190_999;
        let b: u64 = 2_000_000;
        let pool0 = gen_ada_token_pool(100 * LOVELACE, 953011295, a, b, ada_cap);

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

        let mut pool = gen_ada_token_pool(0, TOKEN_EMISSION, a, b, ada_cap);

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
            let Next::Succ(pool3) = pool2.swap(Ask(pool2.ada_cap_thr - pool2.reserves_x.untag())) else {
                panic!()
            };
            let y_max = pool2.reserves_y.untag() - pool3.reserves_y.untag();
            assert_eq!(b, pool2.ada_cap_thr - pool2.reserves_x.untag());
            assert_eq!(q, y_max);
            assert_eq!(pool3.reserves_x.untag(), pool3.ada_cap_thr)
        }
    }
    #[test]
    fn test_available_lq_bug() {
        let ada_cap: u64 = 25_240 * LOVELACE;
        let a: u64 = 174_150_190_999;
        let b: u64 = 2_000_000;
        let pool0 = gen_ada_token_pool(150000000, 933525758, a, b, ada_cap);

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
        let pool0 = gen_ada_token_pool(0, TOKEN_EMISSION, a, b, ada_cap);

        let Next::Succ(pool1) = pool0.swap(Ask(to_dep)) else {
            panic!()
        };
        let y_rec = pool0.reserves_y.untag() - pool1.reserves_y.untag();

        let Some(AvailableLiquidity { input: b, output: q }) = pool0.output_estimation(Ask(to_dep)) else {
            panic!()
        };
        assert_eq!(q, y_rec);
        assert_eq!(b, to_dep);

        let Next::Succ(pool2) = pool1.swap(Bid(y_rec)) else {
            panic!()
        };

        let x_rec = pool1.reserves_x.untag() - pool2.reserves_x.untag();

        let Some(AvailableLiquidity { input: b, output: q }) = pool1.output_estimation(Bid(y_rec)) else {
            panic!()
        };
        assert_eq!(q, x_rec);
        assert_eq!(b, y_rec);

        let too_bid_ask_input = 2 * ada_cap.clone();
        let max_ask_input = (pool2.ada_cap_thr - MIN_ADA) * 101 / 100 - pool2.reserves_x.untag();
        let Some(AvailableLiquidity { input: b, output: q }) =
            pool2.output_estimation(Ask(too_bid_ask_input))
        else {
            panic!()
        };
        let Next::Succ(pool3) = pool2.swap(Ask(max_ask_input)) else {
            panic!()
        };
        let y_max = pool2.reserves_y.untag() - pool3.reserves_y.untag();
        assert_eq!(q, y_max);
        assert_eq!(b, max_ask_input);
    }
}
