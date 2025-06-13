use std::fmt::{Debug, Display, Formatter};
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
use dashu_float::DBig;
use num_rational::Ratio;
use num_traits::ToPrimitive;
use num_traits::{CheckedAdd, CheckedSub};
use primitive_types::U512;
use void::Void;

use bloom_offchain::execution_engine::liquidity_book::core::Next;
use bloom_offchain::execution_engine::liquidity_book::market_maker::AvailableLiquidity;
use bloom_offchain::execution_engine::liquidity_book::market_maker::{
    AbsoluteReserves, MakerBehavior, MarketMaker, PoolQuality, SpotPrice,
};
use bloom_offchain::execution_engine::liquidity_book::side::{OnSide, Side};
use bloom_offchain::execution_engine::liquidity_book::types::AbsolutePrice;
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

use crate::constants::{ADA_WEIGHT, FEE_DEN, MAX_LQ_CAP, TOKEN_WEIGHT, WEIGHT_FEE_DEN};
use crate::data::cfmm_pool::AMMOps;
use crate::data::deposit::ClassicalOnChainDeposit;
use crate::data::operation_output::{DepositOutput, OperationResultBlueprint, RedeemOutput};
use crate::data::order::{Base, PoolNft, Quote};
use crate::data::pair::order_canonical;
use crate::data::pool::{
    ApplyOrder, ApplyOrderError, CFMMPoolAction, ImmutablePoolUtxo, Lq, PoolAssetMapping, PoolValidation, Rx,
    Ry,
};
use crate::data::redeem::ClassicalOnChainRedeem;
use crate::data::PoolId;
use crate::deployment::ProtocolValidator::{
    BalanceFnPoolDeposit, BalanceFnPoolRedeem, BalanceFnPoolV1, BalanceFnPoolV2, ConstFnFeeSwitchPoolDeposit,
    ConstFnFeeSwitchPoolRedeem, ConstFnPoolDeposit, ConstFnPoolRedeem, RoyaltyPoolV1Deposit,
    RoyaltyPoolV1Redeem, RoyaltyPoolV2Deposit, RoyaltyPoolV2Redeem, StableFnPoolT2TDeposit,
    StableFnPoolT2TRedeem,
};
use crate::deployment::{DeployedScriptInfo, DeployedValidator, DeployedValidatorErased, RequiresValidator};
use crate::pool_math::balance_math::balance_cfmm_output_amount;
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

impl Display for BalancePool {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(&*format!(
            "BalancedCFMM(id: {}, static_price: {}, rx: {}, ry: {}, tx: {}, ty: {})",
            self.id,
            self.static_price(),
            self.reserves_x,
            self.reserves_y,
            self.treasury_x,
            self.treasury_y
        ))
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
                *quote_reserves += input;
                *base_reserves -= output;
                *quote_treasury += (input * self.treasury_fee.numer() / self.treasury_fee.denom());
            }
            OnSide::Ask(input) => {
                // User ask is the opposite; sell the base asset for the quote asset.
                *base_reserves += input;
                *quote_reserves -= output;
                *base_treasury += (input * self.treasury_fee.numer() / self.treasury_fee.denom());
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
        if x == base {
            AbsolutePrice::new_unsafe(
                (available_y_reserves * WEIGHT_FEE_DEN) / self.weight_y,
                (available_x_reserves * WEIGHT_FEE_DEN) / self.weight_x,
            )
            .into()
        } else {
            AbsolutePrice::new_unsafe(
                (available_x_reserves * WEIGHT_FEE_DEN) / self.weight_x,
                (available_y_reserves * WEIGHT_FEE_DEN) / self.weight_y,
            )
            .into()
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
    fn available_liquidity_on_side(&self, worst_price: OnSide<AbsolutePrice>) -> Option<AvailableLiquidity> {
        const BN_ONE: BigNumber = BigNumber { value: DBig::ONE };

        const MAX_ERR: i32 = 1;
        const MAX_ITERS: u32 = 25;

        let [base, _] = order_canonical(self.asset_x.untag(), self.asset_y.untag());

        let tradable_x_reserves = BigNumber::from((self.reserves_x - self.treasury_x).untag() as f64);
        let weight_x = BigNumber::from(self.weight_x as f64).div(BigNumber::from(WEIGHT_FEE_DEN as f64));
        let tradable_y_reserves = BigNumber::from((self.reserves_y - self.treasury_y).untag() as f64);
        let weight_y = BigNumber::from(self.weight_y as f64).div(BigNumber::from(WEIGHT_FEE_DEN as f64));
        let raw_fee_x = self.lp_fee_x.checked_sub(&self.treasury_fee)?;
        let fee_x = BigNumber::from(raw_fee_x.to_f64()?);
        let raw_fee_y = self.lp_fee_y.checked_sub(&self.treasury_fee)?;
        let fee_y = BigNumber::from(raw_fee_y.to_f64()?);
        let bid_price = BigNumber::from(*worst_price.unwrap().denom() as f64)
            / BigNumber::from(*worst_price.unwrap().numer() as f64);
        let ask_price = BigNumber::from(*worst_price.unwrap().numer() as f64)
            / BigNumber::from(*worst_price.unwrap().denom() as f64);

        let (
            tradable_reserves_base,
            w_base,
            tradable_reserves_quote,
            w_quote,
            total_fee_mult,
            avg_sell_price,
        ) = match worst_price {
            OnSide::Bid(_) if base == self.asset_x.untag() => (
                tradable_y_reserves,
                weight_y,
                tradable_x_reserves,
                weight_x,
                fee_y,
                bid_price,
            ),
            OnSide::Bid(_) => (
                tradable_x_reserves,
                weight_x,
                tradable_y_reserves,
                weight_y,
                fee_x,
                bid_price,
            ),
            OnSide::Ask(_) if base == self.asset_x.untag() => (
                tradable_x_reserves,
                weight_x,
                tradable_y_reserves,
                weight_y,
                fee_x,
                ask_price,
            ),
            OnSide::Ask(_) => (
                tradable_y_reserves,
                weight_y,
                tradable_x_reserves,
                weight_x,
                fee_y,
                ask_price,
            ),
        };

        let lq_balance =
            tradable_reserves_base.pow(&w_base.clone()) * tradable_reserves_quote.pow(&w_quote.clone());

        //# Constants for calculations:
        let a = (w_base.clone() + w_quote.clone()) / w_quote.clone();
        let b = BN_ONE - a.clone();
        let c = lq_balance.pow(&BN_ONE.div(w_quote.clone())) * w_base.clone() / w_quote.clone();
        let k = c.clone() / b.clone();
        //
        let x0 = tradable_reserves_base.clone();
        let mut x1 = x0.clone().mul(BigNumber::from(1.1)).to_precision(0); //int(1.1 * x0);
        let mut err = tradable_reserves_base.clone();
        let mut counter = 0;
        // // # Numerical calculation procedure (usual less than 5 iterations).
        // // # You can increase 'maxErr' value to decrease number of iters.
        while err.to_precision(10).value.ge(&DBig::from(MAX_ERR)) && counter < MAX_ITERS {
            let f_x = (avg_sell_price.clone().div(total_fee_mult.clone()))
                - k.clone() * (x1.clone().pow(&b.clone()) - x0.clone().pow(&b.clone()))
                    / (x1.clone() - x0.clone());
            let f_x_der = k.clone()
                * ((b.clone() - BN_ONE) * x1.clone().pow(&(b.clone() + BN_ONE))
                    + x1.clone() * x0.clone().pow(&b.clone())
                    - b.clone() * x0.clone() * x1.clone().pow(&b.clone()))
                / (x1.clone() * (x1.clone() - x0.clone()).powi(2));
            let add = f_x.clone().div(f_x_der.clone());

            if (x1.clone() + add.clone()).value.to_f64().value() > 0_f64 {
                x1 = x1.clone() + add.clone();
                err = BigNumber::from(add.clone().value.to_f32().value().abs());
                counter += 1;
            } else {
                break;
            }
        }
        let input_amount = (x1.clone() - tradable_reserves_base.clone()) / total_fee_mult;

        let tradable_reserves_quote_final =
            (lq_balance / x1.clone().pow(&w_base.clone())).pow(&BN_ONE.div(&w_quote));
        let output_amount = tradable_reserves_quote - tradable_reserves_quote_final;

        let input_amount_val = <u64>::try_from(input_amount.value.to_int().value()).ok()?;
        let output_amount_val = <u64>::try_from(output_amount.value.to_int().value()).ok()?;

        Some(AvailableLiquidity {
            input: input_amount_val,
            output: output_amount_val,
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

impl<Ctx> ApplyOrder<ClassicalOnChainDeposit, Ctx> for BalancePool
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

                Ok((
                    self,
                    OperationResultBlueprint::single_output(deposit_output, validator),
                ))
            }
            None => Err(ApplyOrderError::incompatible(deposit)),
        }
    }
}

impl<Ctx> ApplyOrder<ClassicalOnChainRedeem, Ctx> for BalancePool
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

                Ok((
                    self,
                    OperationResultBlueprint::single_output(redeem_output, validator),
                ))
            }
            None => Err(ApplyOrderError::incompatible(redeem)),
        }
    }
}

#[cfg(test)]
mod tests {
    use cml_chain::address::Address;
    use cml_chain::builders::tx_builder::TransactionUnspentOutput;
    use std::cmp::min;

    use cml_chain::assets::AssetBundle;
    use cml_chain::plutus::PlutusData;
    use cml_chain::transaction::{ConwayFormatTxOut, TransactionInput, TransactionOutput};
    use cml_chain::{Deserialize, Value};
    use cml_core::serialization::Serialize;
    use cml_crypto::{Ed25519KeyHash, ScriptHash, TransactionHash};
    use num_rational::Ratio;
    use type_equalities::IsEqual;
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
    use spectrum_offchain::domain::Has;

    use crate::data::balance_pool::{BalancePool, BalancePoolConfig, BalancePoolRedeemer, BalancePoolVer};
    use crate::data::order::ClassicalOrder;
    use crate::data::order::OrderType::BalanceFn;
    use crate::data::pool::{ApplyOrder, CFMMPoolAction, PoolValidation};
    use crate::data::redeem::{ClassicalOnChainRedeem, Redeem};
    use crate::data::{OnChainOrderId, PoolId};
    use crate::deployment::ProtocolValidator::{
        BalanceFnPoolRedeem, ConstFnFeeSwitchPoolRedeem, ConstFnPoolRedeem, RoyaltyPoolV1Redeem,
        RoyaltyPoolV2Redeem, StableFnPoolT2TRedeem,
    };
    use crate::deployment::{DeployedValidator, DeployedValidators, ProtocolScriptHashes};

    const DATUM_SAMPLE: &str = "d8799fd8799f581c5df8fe3f9f0e10855f930e0ea6c227e3bba0aba54d39f9d55b95e21c436e6674ffd8799f4040ff01d8799f581c4b3459fd18a1dbabe207cd19c9951a9fac9f5c0f9c384e3d97efba26457465737443ff04d8799f581c0df79145b95580c14ef4baf8d022d7f0cbb08f3bed43bf97a2ddd8cb426c71ff1a000186820a00009fd8799fd87a9f581cb046b660db0eaf9be4f4300180ccf277e4209dada77c48fbd37ba81dffffff581c8d4be10d934b60a22f267699ea3f7ebdade1f8e535d1bd0ef7ce18b61a0501bced08ff";

    const mock_address: &str = "addr1z8d70g7c58vznyye9guwagdza74x36f3uff0eyk2zwpcpx6c96rgsm7p0hmwrj8e28qny5yxwya63e8gjj8s2ugfglhsxedx9j";

    const TX: &str = "a035c1cb245735680dcb3c46a9a3e692fbf550c8a5d7c4ada1471f97cc92dc55";

    struct Ctx {
        bounds: PoolValidation,
        scripts: ProtocolScriptHashes,
        mock_output: TransactionUnspentOutput,
    }

    const mock_ex_units: ExUnits = ExUnits { mem: 0, steps: 0 };

    impl Has<DeployedValidator<{ ConstFnPoolRedeem as u8 }>> for Ctx {
        fn select<U: IsEqual<DeployedValidator<{ ConstFnPoolRedeem as u8 }>>>(
            &self,
        ) -> DeployedValidator<{ ConstFnPoolRedeem as u8 }> {
            DeployedValidator {
                reference_utxo: self.mock_output.clone(),
                hash: self.scripts.const_fn_pool_redeem.script_hash,
                cost: mock_ex_units,
                marginal_cost: self.scripts.const_fn_pool_redeem.marginal_cost,
            }
        }
    }

    impl Has<DeployedValidator<{ ConstFnFeeSwitchPoolRedeem as u8 }>> for Ctx {
        fn select<U: IsEqual<DeployedValidator<{ ConstFnFeeSwitchPoolRedeem as u8 }>>>(
            &self,
        ) -> DeployedValidator<{ ConstFnFeeSwitchPoolRedeem as u8 }> {
            DeployedValidator {
                reference_utxo: self.mock_output.clone(),
                hash: self.scripts.const_fn_fee_switch_pool_redeem.script_hash,
                cost: mock_ex_units,
                marginal_cost: self.scripts.const_fn_fee_switch_pool_redeem.marginal_cost,
            }
        }
    }

    impl Has<DeployedValidator<{ BalanceFnPoolRedeem as u8 }>> for Ctx {
        fn select<U: IsEqual<DeployedValidator<{ BalanceFnPoolRedeem as u8 }>>>(
            &self,
        ) -> DeployedValidator<{ BalanceFnPoolRedeem as u8 }> {
            DeployedValidator {
                reference_utxo: self.mock_output.clone(),
                hash: self.scripts.balance_fn_pool_redeem.script_hash,
                cost: mock_ex_units,
                marginal_cost: self.scripts.balance_fn_pool_redeem.marginal_cost,
            }
        }
    }

    impl Has<DeployedValidator<{ StableFnPoolT2TRedeem as u8 }>> for Ctx {
        fn select<U: IsEqual<DeployedValidator<{ StableFnPoolT2TRedeem as u8 }>>>(
            &self,
        ) -> DeployedValidator<{ StableFnPoolT2TRedeem as u8 }> {
            DeployedValidator {
                reference_utxo: self.mock_output.clone(),
                hash: self.scripts.stable_fn_pool_t2t_redeem.script_hash,
                cost: mock_ex_units,
                marginal_cost: self.scripts.stable_fn_pool_t2t_redeem.marginal_cost,
            }
        }
    }

    impl Has<DeployedValidator<{ RoyaltyPoolV1Redeem as u8 }>> for Ctx {
        fn select<U: IsEqual<DeployedValidator<{ RoyaltyPoolV1Redeem as u8 }>>>(
            &self,
        ) -> DeployedValidator<{ RoyaltyPoolV1Redeem as u8 }> {
            DeployedValidator {
                reference_utxo: self.mock_output.clone(),
                hash: self.scripts.royalty_pool_redeem.script_hash,
                cost: mock_ex_units,
                marginal_cost: self.scripts.royalty_pool_redeem.marginal_cost,
            }
        }
    }

    impl Has<DeployedValidator<{ RoyaltyPoolV2Redeem as u8 }>> for Ctx {
        fn select<U: IsEqual<DeployedValidator<{ RoyaltyPoolV2Redeem as u8 }>>>(
            &self,
        ) -> DeployedValidator<{ RoyaltyPoolV2Redeem as u8 }> {
            DeployedValidator {
                reference_utxo: self.mock_output.clone(),
                hash: self.scripts.royalty_pool_redeem.script_hash,
                cost: mock_ex_units,
                marginal_cost: self.scripts.royalty_pool_redeem.marginal_cost,
            }
        }
    }

    impl Has<PoolValidation> for Ctx {
        fn select<U: IsEqual<PoolValidation>>(&self) -> PoolValidation {
            self.bounds
        }
    }

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
            weight_x: 1,
            reserves_y: TaggedAmount::new(reserves_y),
            weight_y: 4,
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

        let raw_deployment = std::fs::read_to_string("/Users/oskin/dev/spectrum/spectrum-offchain-multiplatform/bloom-cardano-agent/resources/mainnet.deployment.json").expect("Cannot load deployment file");
        let deployment: DeployedValidators =
            serde_json::from_str(&raw_deployment).expect("Invalid deployment file");
        let scripts = ProtocolScriptHashes::from(&deployment);

        let mock_input: TransactionInput = TransactionInput {
            transaction_id: TransactionHash::from_hex(TX).unwrap(),
            index: 0,
            encodings: None,
        };

        let mock_output: TransactionOutput = TransactionOutput::ConwayFormatTxOut(ConwayFormatTxOut {
            address: Address::from_bech32(mock_address).unwrap(),
            amount: Value::new(1000000, AssetBundle::new()),
            datum_option: None,
            script_reference: None,
            encodings: None,
        });

        let mock_unspent_output: TransactionUnspentOutput = TransactionUnspentOutput {
            input: mock_input,
            output: mock_output,
        };

        let ctx = Ctx {
            scripts,
            bounds: PoolValidation {
                min_n2t_lovelace: 150_000_000,
                min_t2t_lovelace: 10_000_000,
            },
            mock_output: mock_unspent_output,
        };

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

        let test = pool.apply_order(test_order, ctx);

        let res = test.map(|res| println!("{:?}", res.0));

        assert_eq!(1, 1)
    }

    #[test]
    fn available_liquidity_test() {
        let pool = gen_ada_token_pool(2105999997, 1981759952, 9223372036854587823, 99000, 99000, 0, 0, 0);

        let worst_price = AbsolutePrice::new(8361312554391071, 36028797018963968).unwrap();
        let Some(AvailableLiquidity {
            input: _,
            output: quote_qty_ask_spot,
        }) = pool.available_liquidity_on_side(Ask(worst_price))
        else {
            !panic!()
        };

        let worst_price = AbsolutePrice::new(1125899906842624, 4717703533773517).unwrap();
        let Some(AvailableLiquidity {
            input: _,
            output: quote_qty_bid_spot,
        }) = pool.available_liquidity_on_side(Bid(worst_price))
        else {
            !panic!()
        };

        assert_eq!(quote_qty_ask_spot, 2813733);
        assert_eq!(quote_qty_bid_spot, 14477946)
    }
}
