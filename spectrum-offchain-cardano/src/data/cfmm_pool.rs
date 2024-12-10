use std::fmt::Debug;
use std::ops::{Div, Neg};

use bignumber::BigNumber;
use cml_chain::address::Address::Enterprise;
use cml_chain::address::{Address, EnterpriseAddress};
use cml_chain::assets::MultiAsset;
use cml_chain::certs::{Credential, StakeCredential};
use cml_chain::plutus::{ConstrPlutusData, PlutusData};
use cml_chain::transaction::{ConwayFormatTxOut, DatumOption, TransactionOutput};
use cml_chain::utils::BigInteger;
use cml_chain::Value;
use cml_core::serialization::{RawBytesEncoding, Serialize, ToBytes};
use cml_crypto::{Ed25519Signature, PublicKey, ScriptHash};
use num_rational::Ratio;
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
use spectrum_cardano_lib::address::{AddressExtension, InlineCredential, PlutusAddress, PlutusCredential};
use spectrum_cardano_lib::ex_units::ExUnits;
use spectrum_cardano_lib::plutus_data::{
    ConstrPlutusDataExtension, DatumExtension, IntoPlutusData, PlutusDataExtension,
};
use spectrum_cardano_lib::transaction::TransactionOutputExtension;
use spectrum_cardano_lib::types::TryFromPData;
use spectrum_cardano_lib::value::ValueExtension;
use spectrum_cardano_lib::AssetClass::Native;
use spectrum_cardano_lib::{Ed25519PublicKey, NetworkId, TaggedAmount, TaggedAssetClass, Token};
use spectrum_offchain::domain::{Has, Stable};
use spectrum_offchain::ledger::{IntoLedger, TryFromLedger};

use crate::constants::{FEE_DEN, LEGACY_FEE_NUM_MULTIPLIER, MAX_LQ_CAP, POOL_OUT_IDX_IN};
use crate::data::dao_request::{DAOContext, DaoAction, DaoRequestDataToSign, OnChainDAOActionRequest};
use crate::data::deposit::ClassicalOnChainDeposit;
use crate::data::fee_switch_bidirectional_fee::FeeSwitchBidirectionalPoolConfig;
use crate::data::fee_switch_pool::FeeSwitchPoolConfig;
use crate::data::limit_swap::ClassicalOnChainLimitSwap;
use crate::data::operation_output::DaoActionResult::{RequestorOutput, TreasuryWithdraw};
use crate::data::operation_output::OperationResultOutputs::SingleOutput;
use crate::data::operation_output::{
    ContexBasedRedeemerCreator, DaoActionResult, DaoRequestorOutput, DepositOutput, OperationResultBlueprint,
    OperationResultContext, OperationResultOutputs, RedeemOutput, RoyaltyWithdrawOutput, SwapOutput,
    TreasuryWithdrawOutput,
};
use crate::data::order::{Base, ClassicalOrder, PoolNft, Quote};
use crate::data::pair::order_canonical;
use crate::data::pool::{
    ApplyOrder, ApplyOrderError, ImmutablePoolUtxo, Lq, PoolAssetMapping, PoolValidation, Rx, Ry,
};
use crate::data::redeem::ClassicalOnChainRedeem;
use crate::data::royalty_pool::{RoyaltyPoolConfig, ROYALTY_DATUM_MAPPING};
use crate::data::royalty_withdraw_request::{DataToSign, OnChainRoyaltyWithdraw, RoyaltyWithdrawContext};
use crate::data::PoolId;
use crate::deployment::ProtocolValidator::{
    BalanceFnPoolDeposit, BalanceFnPoolRedeem, ConstFnFeeSwitchPoolDeposit, ConstFnFeeSwitchPoolRedeem,
    ConstFnFeeSwitchPoolSwap, ConstFnPoolDeposit, ConstFnPoolFeeSwitch, ConstFnPoolFeeSwitchBiDirFee,
    ConstFnPoolFeeSwitchV2, ConstFnPoolRedeem, ConstFnPoolSwap, ConstFnPoolV1, ConstFnPoolV2,
    RoyaltyPoolDAOV1, RoyaltyPoolDAOV1Request, RoyaltyPoolRoyaltyWithdraw, RoyaltyPoolV1,
    RoyaltyPoolV1Deposit, RoyaltyPoolV1Redeem, RoyaltyPoolV1RoyaltyWithdrawRequest, StableFnPoolT2TDeposit,
    StableFnPoolT2TRedeem,
};
use crate::deployment::{DeployedScriptInfo, DeployedValidator, DeployedValidatorErased, RequiresValidator};
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
    FeeSwitchV2,
    FeeSwitchBiDirFee,
    RoyaltyPoolV1,
}

impl ConstFnPoolVer {
    pub fn try_from_address<Ctx>(pool_addr: &Address, ctx: &Ctx) -> Option<ConstFnPoolVer>
    where
        Ctx: Has<DeployedScriptInfo<{ ConstFnPoolV1 as u8 }>>
            + Has<DeployedScriptInfo<{ ConstFnPoolV2 as u8 }>>
            + Has<DeployedScriptInfo<{ ConstFnPoolFeeSwitch as u8 }>>
            + Has<DeployedScriptInfo<{ ConstFnPoolFeeSwitchV2 as u8 }>>
            + Has<DeployedScriptInfo<{ ConstFnPoolFeeSwitchBiDirFee as u8 }>>
            + Has<DeployedScriptInfo<{ RoyaltyPoolV1 as u8 }>>,
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
                .select::<DeployedScriptInfo<{ ConstFnPoolFeeSwitchV2 as u8 }>>()
                .script_hash
                == *this_hash
            {
                return Some(ConstFnPoolVer::FeeSwitchV2);
            } else if ctx
                .select::<DeployedScriptInfo<{ ConstFnPoolFeeSwitchBiDirFee as u8 }>>()
                .script_hash
                == *this_hash
            {
                return Some(ConstFnPoolVer::FeeSwitchBiDirFee);
            } else if ctx
                .select::<DeployedScriptInfo<{ RoyaltyPoolV1 as u8 }>>()
                .script_hash
                == *this_hash
            {
                return Some(ConstFnPoolVer::RoyaltyPoolV1);
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
    pub royalty_fee: Ratio<u64>,
    pub royalty_x: TaggedAmount<Rx>,
    pub royalty_y: TaggedAmount<Ry>,
    pub lq_lower_bound: TaggedAmount<Rx>,
    pub admin_address: ScriptHash,
    pub treasury_address: ScriptHash,
    pub royalty_pub_key: Ed25519PublicKey,
    pub ver: ConstFnPoolVer,
    pub marginal_cost: ExUnits,
    pub bounds: PoolValidation,
    pub nonce: u64,
    pub stake_part_script_hash: Option<ScriptHash>,
}

impl ConstFnPool {
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
            self.reserves_x - self.treasury_x - self.royalty_x,
            self.reserves_y - self.treasury_y - self.royalty_y,
            base_asset,
            base_amount,
            self.lp_fee_x - self.treasury_fee - self.royalty_fee,
            self.lp_fee_y - self.treasury_fee - self.royalty_fee,
        )
    }

    fn reward_lp(
        &self,
        in_x_amount: u64,
        in_y_amount: u64,
    ) -> Option<(TaggedAmount<Lq>, TaggedAmount<Rx>, TaggedAmount<Ry>)> {
        classic_cfmm_reward_lp(
            self.reserves_x - self.treasury_x - self.royalty_x,
            self.reserves_y - self.treasury_y - self.royalty_y,
            self.liquidity,
            in_x_amount,
            in_y_amount,
        )
    }

    fn shares_amount(&self, burned_lq: TaggedAmount<Lq>) -> Option<(TaggedAmount<Rx>, TaggedAmount<Ry>)> {
        classic_cfmm_shares_amount(
            self.reserves_x - self.treasury_x - self.royalty_x,
            self.reserves_y - self.treasury_y - self.royalty_y,
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
        + Has<DeployedValidator<{ ConstFnPoolFeeSwitchV2 as u8 }>>
        + Has<DeployedValidator<{ ConstFnPoolFeeSwitchBiDirFee as u8 }>>
        + Has<DeployedValidator<{ RoyaltyPoolV1 as u8 }>>,
{
    fn get_validator(&self, ctx: &Ctx) -> DeployedValidatorErased {
        match self.ver {
            ConstFnPoolVer::V1 => ctx
                .select::<DeployedValidator<{ ConstFnPoolV1 as u8 }>>()
                .erased(),
            ConstFnPoolVer::FeeSwitch => ctx
                .select::<DeployedValidator<{ ConstFnPoolFeeSwitch as u8 }>>()
                .erased(),
            ConstFnPoolVer::FeeSwitchV2 => ctx
                .select::<DeployedValidator<{ ConstFnPoolFeeSwitchV2 as u8 }>>()
                .erased(),
            ConstFnPoolVer::FeeSwitchBiDirFee => ctx
                .select::<DeployedValidator<{ ConstFnPoolFeeSwitchBiDirFee as u8 }>>()
                .erased(),
            ConstFnPoolVer::RoyaltyPoolV1 => ctx
                .select::<DeployedValidator<{ RoyaltyPoolV1 as u8 }>>()
                .erased(),
            _ => ctx
                .select::<DeployedValidator<{ ConstFnPoolV2 as u8 }>>()
                .erased(),
        }
    }
}

impl MakerBehavior for ConstFnPool {
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
        let (base_reserves, base_treasury, base_royalty, quote_reserves, quote_treasury, quote_royalty) =
            if x == base {
                (
                    self.reserves_x.as_mut(),
                    self.treasury_x.as_mut(),
                    self.royalty_x.as_mut(),
                    self.reserves_y.as_mut(),
                    self.treasury_y.as_mut(),
                    self.royalty_y.as_mut(),
                )
            } else {
                (
                    self.reserves_y.as_mut(),
                    self.treasury_y.as_mut(),
                    self.royalty_y.as_mut(),
                    self.reserves_x.as_mut(),
                    self.treasury_x.as_mut(),
                    self.royalty_x.as_mut(),
                )
            };
        match input {
            OnSide::Bid(input) => {
                // A user bid means that they wish to buy the base asset for the quote asset, hence
                // pool reserves of base decreases while reserves of quote increase.
                *quote_reserves += input;
                *base_reserves -= output;
                *quote_treasury += (input * self.treasury_fee.numer()) / self.treasury_fee.denom();
                *quote_royalty += (input * self.royalty_fee.numer()) / self.royalty_fee.denom();
            }
            OnSide::Ask(input) => {
                // User ask is the opposite; sell the base asset for the quote asset.
                *base_reserves += input;
                *quote_reserves -= output;
                *base_treasury += (input * self.treasury_fee.numer()) / self.treasury_fee.denom();
                *base_royalty += (input * self.royalty_fee.numer()) / self.royalty_fee.denom();
            }
        }
        Next::Succ(self)
    }
}

impl MarketMaker for ConstFnPool {
    type U = ExUnits;

    fn static_price(&self) -> SpotPrice {
        let x = self.asset_x.untag();
        let y = self.asset_y.untag();
        let [base, _] = order_canonical(x, y);
        let available_x_reserves = (self.reserves_x - self.treasury_x - self.royalty_x).untag();
        let available_y_reserves = (self.reserves_y - self.treasury_y - self.royalty_y).untag();
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

        let tradable_x_reserves =
            BigNumber::from((self.reserves_x - self.treasury_x - self.royalty_x).untag() as f64);
        let tradable_y_reserves =
            BigNumber::from((self.reserves_y - self.treasury_y - self.royalty_y).untag() as f64);
        let raw_fee_x = self
            .lp_fee_x
            .checked_sub(&self.treasury_fee)
            .and_then(|fee| fee.checked_sub(&self.royalty_fee))?;
        let fee_x = BigNumber::from(raw_fee_x.to_f64()?);
        let raw_fee_y = self
            .lp_fee_y
            .checked_sub(&self.treasury_fee)
            .and_then(|fee| fee.checked_sub(&self.royalty_fee))?;
        let fee_y = BigNumber::from(raw_fee_y.to_f64()?);
        let bid_price = BigNumber::from(*worst_price.unwrap().denom() as f64)
            / BigNumber::from(*worst_price.unwrap().numer() as f64);
        let ask_price = BigNumber::from(*worst_price.unwrap().numer() as f64)
            / BigNumber::from(*worst_price.unwrap().denom() as f64);

        let (tradable_reserves_base, tradable_reserves_quote, total_fee_mult, avg_price) = match worst_price {
            OnSide::Bid(_) if base == self.asset_x.untag() => {
                (tradable_y_reserves, tradable_x_reserves, fee_y, bid_price)
            }
            OnSide::Bid(_) => (tradable_x_reserves, tradable_y_reserves, fee_x, bid_price),
            OnSide::Ask(_) if base == self.asset_x.untag() => {
                (tradable_x_reserves, tradable_y_reserves, fee_x, ask_price)
            }
            OnSide::Ask(_) => (tradable_y_reserves, tradable_x_reserves, fee_y, ask_price),
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

impl<Ctx> TryFromLedger<TransactionOutput, Ctx> for ConstFnPool
where
    Ctx: Has<DeployedScriptInfo<{ ConstFnPoolV1 as u8 }>>
        + Has<DeployedScriptInfo<{ ConstFnPoolV2 as u8 }>>
        + Has<DeployedScriptInfo<{ ConstFnPoolFeeSwitch as u8 }>>
        + Has<DeployedScriptInfo<{ ConstFnPoolFeeSwitchV2 as u8 }>>
        + Has<DeployedScriptInfo<{ ConstFnPoolFeeSwitchBiDirFee as u8 }>>
        + Has<DeployedScriptInfo<{ RoyaltyPoolV1 as u8 }>>
        + Has<PoolValidation>,
{
    fn try_from_ledger(repr: &TransactionOutput, ctx: &Ctx) -> Option<Self> {
        if let Some(pool_ver) = ConstFnPoolVer::try_from_address(repr.address(), ctx) {
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
                ConstFnPoolVer::FeeSwitchV2 => {
                    ctx.select::<DeployedScriptInfo<{ ConstFnPoolFeeSwitchV2 as u8 }>>()
                        .marginal_cost
                }
                ConstFnPoolVer::FeeSwitchBiDirFee => {
                    ctx.select::<DeployedScriptInfo<{ ConstFnPoolFeeSwitchBiDirFee as u8 }>>()
                        .marginal_cost
                }
                ConstFnPoolVer::RoyaltyPoolV1 => {
                    ctx.select::<DeployedScriptInfo<{ RoyaltyPoolV1 as u8 }>>()
                        .marginal_cost
                }
            };
            match pool_ver {
                ConstFnPoolVer::V1 | ConstFnPoolVer::V2 => {
                    let conf = LegacyCFMMPoolConfig::try_from_pd(pd.clone())?;
                    let liquidity_neg = value.amount_of(conf.asset_lq.into())?;
                    return Some(ConstFnPool {
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
                        royalty_fee: Ratio::new_raw(0, 100),
                        royalty_x: TaggedAmount::new(0),
                        royalty_y: TaggedAmount::new(0),
                        lq_lower_bound: conf.lq_lower_bound,
                        admin_address: ScriptHash::from([0; 28]),
                        treasury_address: ScriptHash::from([0; 28]),
                        royalty_pub_key: Ed25519PublicKey::from([0; 32]),
                        ver: pool_ver,
                        marginal_cost,
                        bounds,
                        nonce: 0,
                        stake_part_script_hash: stake_part,
                    });
                }
                ConstFnPoolVer::FeeSwitch | ConstFnPoolVer::FeeSwitchV2 => {
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
                        return Some(ConstFnPool {
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
                            royalty_fee: Ratio::new_raw(0, 100),
                            royalty_x: TaggedAmount::new(0),
                            royalty_y: TaggedAmount::new(0),
                            lq_lower_bound: conf.lq_lower_bound,
                            admin_address: ScriptHash::from([0; 28]),
                            treasury_address: ScriptHash::from([0; 28]),
                            royalty_pub_key: Ed25519PublicKey::from([0; 32]),
                            ver: pool_ver,
                            marginal_cost,
                            bounds,
                            nonce: 0,
                            stake_part_script_hash: stake_part,
                        });
                    }
                }
                ConstFnPoolVer::FeeSwitchBiDirFee => {
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
                        return Some(ConstFnPool {
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
                            royalty_fee: Ratio::new_raw(0, 100),
                            royalty_x: TaggedAmount::new(0),
                            royalty_y: TaggedAmount::new(0),
                            lq_lower_bound: conf.lq_lower_bound,
                            admin_address: ScriptHash::from([0; 28]),
                            treasury_address: ScriptHash::from([0; 28]),
                            royalty_pub_key: Ed25519PublicKey::from([0; 32]),
                            ver: pool_ver,
                            marginal_cost,
                            bounds,
                            nonce: 0,
                            stake_part_script_hash: stake_part,
                        });
                    }
                }
                ConstFnPoolVer::RoyaltyPoolV1 => {
                    let conf = RoyaltyPoolConfig::try_from_pd(pd.clone())?;
                    let liquidity_neg = value.amount_of(conf.asset_lq.into())?;
                    let lov = value.amount_of(Native)?;
                    let reserves_x = value.amount_of(conf.asset_x.into())?;
                    let reserves_y = value.amount_of(conf.asset_y.into())?;
                    let pure_reserves_x = reserves_x - conf.treasury_x - conf.royalty_x;
                    let pure_reserves_y = reserves_y - conf.treasury_y - conf.royalty_y;
                    let non_empty_reserves = pure_reserves_x > 0 && pure_reserves_y > 0;
                    let sufficient_lovelace = conf.asset_x.is_native()
                        || conf.asset_y.is_native()
                        || bounds.min_t2t_lovelace <= lov;
                    let enough_reserves = reserves_x > (conf.treasury_x + conf.royalty_x)
                        && reserves_y > (conf.treasury_y + conf.royalty_y);
                    if non_empty_reserves && sufficient_lovelace && enough_reserves {
                        return Some(ConstFnPool {
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
                            royalty_fee: Ratio::new_raw(conf.royalty_fee_num, FEE_DEN),
                            royalty_x: TaggedAmount::new(conf.royalty_x),
                            royalty_y: TaggedAmount::new(conf.royalty_y),
                            lq_lower_bound: TaggedAmount::new(0),
                            admin_address: conf.admin_address,
                            treasury_address: ScriptHash::from_raw_bytes(&conf.treasury_address).ok()?,
                            royalty_pub_key: conf.royalty_pub_key.try_into().ok()?,
                            ver: pool_ver,
                            marginal_cost,
                            bounds,
                            nonce: conf.nonce,
                            stake_part_script_hash: stake_part,
                        });
                    }
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

        if self.ver == ConstFnPoolVer::FeeSwitch || self.ver == ConstFnPoolVer::FeeSwitchV2 {
            if let Some(DatumOption::Datum { datum, .. }) = &mut immut_pool.datum_option {
                unsafe_update_pd_fee_switch(datum, self.treasury_x.untag(), self.treasury_y.untag());
            }
        } else if self.ver == ConstFnPoolVer::RoyaltyPoolV1 {
            if let Some(DatumOption::Datum { datum, .. }) = &mut immut_pool.datum_option {
                unsafe_update_pd_royalty(
                    datum,
                    // royalty pool have the same lp_fee_x and lp_fee_y values
                    *self.lp_fee_x.numer(),
                    *self.treasury_fee.numer(),
                    *self.royalty_fee.numer(),
                    self.treasury_x.untag(),
                    self.treasury_y.untag(),
                    self.royalty_x.untag(),
                    self.royalty_y.untag(),
                    self.treasury_address,
                    self.admin_address,
                    self.nonce,
                );
            }
        }

        if let (Some(stake_part_script_hash)) = self.stake_part_script_hash {
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

pub fn unsafe_update_pd_fee_switch(data: &mut PlutusData, treasury_x: u64, treasury_y: u64) {
    let cpd = data.get_constr_pd_mut().unwrap();
    cpd.set_field(6, treasury_x.into_pd());
    cpd.set_field(7, treasury_y.into_pd());
}

pub fn unsafe_update_pd_royalty(
    data: &mut PlutusData,
    lp_fee_num: u64,
    treasury_fee_num: u64,
    royalty_fee_num: u64,
    treasury_x: u64,
    treasury_y: u64,
    royalty_x: u64,
    royalty_y: u64,
    treasury_address: ScriptHash,
    admin_address: ScriptHash,
    new_nonce: u64,
) {
    let admin_addresses: Vec<InlineCredential> = vec![admin_address.into()];

    let cpd = data.get_constr_pd_mut().unwrap();
    cpd.set_field(ROYALTY_DATUM_MAPPING.lp_fee_num, lp_fee_num.into_pd());
    cpd.set_field(ROYALTY_DATUM_MAPPING.treasury_fee_num, treasury_fee_num.into_pd());
    cpd.set_field(ROYALTY_DATUM_MAPPING.royalty_fee_num, royalty_fee_num.into_pd());
    cpd.set_field(ROYALTY_DATUM_MAPPING.treasury_x, treasury_x.into_pd());
    cpd.set_field(ROYALTY_DATUM_MAPPING.treasury_y, treasury_y.into_pd());
    cpd.set_field(ROYALTY_DATUM_MAPPING.royalty_x, royalty_x.into_pd());
    cpd.set_field(ROYALTY_DATUM_MAPPING.royalty_y, royalty_y.into_pd());
    cpd.set_field(ROYALTY_DATUM_MAPPING.admin_address, admin_addresses.into_pd());
    cpd.set_field(
        ROYALTY_DATUM_MAPPING.treasury_address,
        treasury_address.to_raw_bytes().to_vec().into_pd(),
    );
    cpd.set_field(ROYALTY_DATUM_MAPPING.royalty_nonce, new_nonce.into_pd());
}

impl<Ctx> ApplyOrder<ClassicalOnChainLimitSwap, Ctx> for ConstFnPool
where
    Ctx: Has<DeployedValidator<{ ConstFnFeeSwitchPoolSwap as u8 }>>
        + Has<DeployedValidator<{ ConstFnPoolSwap as u8 }>>,
{
    type Result = SwapOutput;

    fn apply_order(
        mut self,
        ord: ClassicalOnChainLimitSwap,
        ctx: Ctx,
    ) -> Result<(Self, OperationResultBlueprint<SwapOutput>), ApplyOrderError<ClassicalOnChainLimitSwap>>
    {
        let ClassicalOrder { id, pool_id, order } = ord.clone();
        let validator = ord.get_validator(&ctx);

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
            let additional_royalty_y = (((order.base_amount.untag() as u128)
                * (*self.royalty_fee.numer() as u128))
                / (*self.royalty_fee.denom() as u128)) as u64;
            self.reserves_x = self.reserves_x - quote_amount.retag();
            self.treasury_y = self.treasury_y + TaggedAmount::new(additional_treasury_y);
            self.royalty_y = self.royalty_y + TaggedAmount::new(additional_royalty_y);
            self.reserves_y = self.reserves_y + order.base_amount.retag();
        } else {
            let additional_treasury_x = (((order.base_amount.untag() as u128)
                * (*self.treasury_fee.numer() as u128))
                / (*self.treasury_fee.denom() as u128)) as u64;
            let additional_royalty_x = (((order.base_amount.untag() as u128)
                * (*self.royalty_fee.numer() as u128))
                / (*self.royalty_fee.denom() as u128)) as u64;
            self.treasury_x = self.treasury_x + TaggedAmount::new(additional_treasury_x);
            self.royalty_x = self.royalty_x + TaggedAmount::new(additional_royalty_x);
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
        Ok((
            self,
            OperationResultBlueprint::single_output(swap_output, validator),
        ))
    }
}

impl<Ctx> ApplyOrder<ClassicalOnChainDeposit, Ctx> for ConstFnPool
where
    Ctx: Has<DeployedValidator<{ ConstFnFeeSwitchPoolDeposit as u8 }>>
        + Has<DeployedValidator<{ ConstFnPoolDeposit as u8 }>>
        + Has<DeployedValidator<{ BalanceFnPoolDeposit as u8 }>>
        + Has<DeployedValidator<{ StableFnPoolT2TDeposit as u8 }>>
        + Has<DeployedValidator<{ RoyaltyPoolV1Deposit as u8 }>>,
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

impl<Ctx> ApplyOrder<ClassicalOnChainRedeem, Ctx> for ConstFnPool
where
    Ctx: Has<DeployedValidator<{ ConstFnFeeSwitchPoolRedeem as u8 }>>
        + Has<DeployedValidator<{ ConstFnPoolRedeem as u8 }>>
        + Has<DeployedValidator<{ BalanceFnPoolRedeem as u8 }>>
        + Has<DeployedValidator<{ StableFnPoolT2TRedeem as u8 }>>
        + Has<DeployedValidator<{ RoyaltyPoolV1Redeem as u8 }>>,
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
        + Has<DeployedValidator<{ RoyaltyPoolV1RoyaltyWithdrawRequest as u8 }>>
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
        let order = royalty_withdraw.order.clone();
        let order_validator = royalty_withdraw.get_validator(&ctx);
        let pool_order_validator: DeployedValidator<{ RoyaltyPoolRoyaltyWithdraw as u8 }> = ctx.get();
        let royalty_withdraw_context: RoyaltyWithdrawContext = ctx.get();

        let data_to_sign = DataToSign {
            withdraw_data: royalty_withdraw.clone().order.into(),
            pool_nonce: self.nonce,
        };

        let mut message_to_sign = order.additional_bytes.clone();
        message_to_sign.extend(&data_to_sign.into_pd().to_cbor_bytes());

        let public_key: PublicKey = self.royalty_pub_key.try_into().unwrap();

        let signature_is_correct = public_key.verify(&message_to_sign, &order.signature);

        if !signature_is_correct {
            return Err(ApplyOrderError::verification_failed(
                royalty_withdraw,
                format!("signature_is_correct: {}", signature_is_correct),
            ));
        }

        self.royalty_x = self
            .royalty_x
            .checked_sub(&order.withdraw_royalty_x)
            .ok_or(ApplyOrderError::incompatible(royalty_withdraw.clone()))?;

        self.royalty_y = self
            .royalty_y
            .checked_sub(&order.withdraw_royalty_y)
            .ok_or(ApplyOrderError::incompatible(royalty_withdraw.clone()))?;

        self.reserves_x = self
            .reserves_x
            .checked_sub(&order.withdraw_royalty_x)
            .ok_or(ApplyOrderError::incompatible(royalty_withdraw.clone()))?;

        self.reserves_y = self
            .reserves_y
            .checked_sub(&order.withdraw_royalty_y)
            .ok_or(ApplyOrderError::incompatible(royalty_withdraw.clone()))?;

        self.nonce += 1;

        let royalty_output = RoyaltyWithdrawOutput {
            token_x_asset: self.asset_x,
            token_x_amount: order.withdraw_royalty_x,
            token_y_asset: self.asset_y,
            token_y_amount: order.withdraw_royalty_y,
            ada_residue: order.init_ada_value - order.fee,
            redeemer_pkh: order.royalty_pub_key_hash,
        };

        let blueprint = OperationResultBlueprint {
            outputs: SingleOutput(royalty_output),
            witness_script: Some((
                pool_order_validator.clone().erased(),
                ContexBasedRedeemerCreator::create(move |context: OperationResultContext| {
                    ConstrPlutusData::new(
                        0,
                        vec![
                            PlutusData::Integer(BigInteger::from(context.pool_input_idx)),
                            PlutusData::Integer(BigInteger::from(context.order_input_idx)),
                        ],
                    )
                    .into_pd()
                }),
            )),
            order_script_validator: order_validator,
            strict_fee: Some(royalty_withdraw_context.transaction_fee),
        };

        Ok((self, blueprint))
    }
}

impl<Ctx> ApplyOrder<OnChainDAOActionRequest, Ctx> for ConstFnPool
where
    Ctx: Has<DeployedValidator<{ RoyaltyPoolDAOV1 as u8 }>>
        + Has<DeployedValidator<{ RoyaltyPoolDAOV1Request as u8 }>>
        + Has<DeployedValidator<{ RoyaltyPoolV1 as u8 }>>
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
        let validator = dao_request.get_validator(&ctx);
        let dao_validator: DeployedValidator<{ RoyaltyPoolDAOV1 as u8 }> = ctx.get();
        let pool_validator: DeployedValidator<{ RoyaltyPoolV1 as u8 }> = ctx.get();
        let dao_ctx: DAOContext = ctx.get();

        let data_to_sign: DaoRequestDataToSign = DaoRequestDataToSign {
            dao_action: dao_request.order.dao_action,
            pool_nft: dao_request.order.pool_nft,
            pool_fee: *dao_request.order.pool_fee.numer(),
            treasury_fee: *dao_request.order.treasury_fee.numer(),
            admin_address: vec![dao_request.order.admin_address.into()],
            pool_address: PlutusAddress {
                payment_cred: PlutusCredential::Script(pool_validator.hash),
                stake_cred: dao_request.order.pool_stake_script_hash.map(Into::into),
            },
            treasury_address: dao_request.order.treasury_address,
            // on contract side we are validating delta between `new_treasury_x` and
            // `prev_treasury_x`
            treasury_x_delta: (dao_request.order.treasury_x_abs_delta.untag() as i64).neg(),
            treasury_y_delta: (dao_request.order.treasury_y_abs_delta.untag() as i64).neg(),
            pool_nonce: self.nonce,
        };

        let data_to_sign_raw = data_to_sign.clone().into_pd().to_cbor_bytes();

        let data_to_sign_with_additional_bytes: Vec<Vec<u8>> = dao_request
            .clone()
            .order
            .additional_bytes
            .into_iter()
            .map(|mut additional_bytes| {
                let mut to_add = data_to_sign_raw.clone();
                additional_bytes.append(&mut to_add);
                additional_bytes
            })
            .collect();

        let mut siq_qty = 0;

        let correct_signatures_qty = dao_request
            .clone()
            .order
            .signatures
            .into_iter()
            .zip::<Vec<[u8; 32]>>(dao_ctx.clone().public_keys.into())
            .zip(data_to_sign_with_additional_bytes)
            .fold(
                0,
                |correct_signatures, (signature_with_public_key, data_to_sign_for_user)| {
                    siq_qty += 1;
                    let (signature, public_key_raw) = signature_with_public_key;
                    if let Some(public_key) = PublicKey::from_raw_bytes(&public_key_raw).ok() {
                        if public_key.verify(&data_to_sign_for_user, &signature) {
                            return correct_signatures + 1;
                        }
                    }
                    correct_signatures
                },
            );

        if correct_signatures_qty < dao_ctx.signature_threshold {
            return Err(ApplyOrderError::verification_failed(
                dao_request.clone(),
                format!(
                    "Incorrect signature threshold. Correct signatures qty: {}. Threshold: {}",
                    correct_signatures_qty, dao_ctx.signature_threshold
                ),
            ));
        }

        let network_id = ctx.select::<NetworkId>();
        match dao_request.order.dao_action {
            DaoAction::WithdrawTreasury => {
                self.reserves_x = self
                    .reserves_x
                    .checked_sub(&dao_request.order.treasury_x_abs_delta)
                    .ok_or(ApplyOrderError::incompatible(dao_request.clone()))?;
                self.treasury_x = self
                    .treasury_x
                    .checked_sub(&dao_request.order.treasury_x_abs_delta)
                    .ok_or(ApplyOrderError::incompatible(dao_request.clone()))?;
                self.reserves_y = self
                    .reserves_y
                    .checked_sub(&dao_request.order.treasury_y_abs_delta)
                    .ok_or(ApplyOrderError::incompatible(dao_request.clone()))?;
                self.treasury_y = self
                    .treasury_y
                    .checked_sub(&dao_request.order.treasury_y_abs_delta)
                    .ok_or(ApplyOrderError::incompatible(dao_request.clone()))?
            }
            DaoAction::ChangeStakePart => {
                self.stake_part_script_hash = dao_request.order.pool_stake_script_hash
            }
            DaoAction::ChangeTreasuryFee => self.treasury_fee = dao_request.order.treasury_fee,
            DaoAction::ChangeTreasuryAddress => self.treasury_address = dao_request.order.treasury_address,
            DaoAction::ChangeAdminAddress => self.admin_address = dao_request.order.admin_address,
            DaoAction::ChangePoolFee => {
                self.lp_fee_x = dao_request.order.pool_fee;
                self.lp_fee_y = dao_request.order.pool_fee
            }
        };

        let requestor_output = RequestorOutput(DaoRequestorOutput {
            lovelace_qty: dao_request
                .order
                .lovelace
                .checked_sub(dao_request.order.fee)
                .ok_or(ApplyOrderError::incompatible(dao_request.clone()))?,
            requestor_address: Enterprise(EnterpriseAddress::new(
                network_id.into(),
                Credential::new_pub_key(dao_request.order.requestor_pkh),
            )),
        });

        self.nonce += 1;

        let redeemer_creator = |pool_id: PoolId,
                                signatures: Vec<Ed25519Signature>,
                                additional_bytes: Vec<Vec<u8>>,
                                dao_action: DaoAction| {
            ContexBasedRedeemerCreator::create(move |context: OperationResultContext| {
                PlutusData::ConstrPlutusData(ConstrPlutusData::new(
                    0,
                    vec![
                        dao_action.into_pd(),
                        context.pool_input_idx.into_pd(),
                        POOL_OUT_IDX_IN.into_pd(),
                        PlutusData::new_list(
                            signatures
                                .iter()
                                .map(|signature| {
                                    PlutusData::new_bytes(signature.clone().to_raw_bytes().to_vec())
                                })
                                .collect(),
                        ),
                        PlutusData::new_list(
                            additional_bytes
                                .iter()
                                .map(|additional_bytes_to_add| {
                                    PlutusData::new_bytes(additional_bytes_to_add.clone())
                                })
                                .collect(),
                        ),
                    ],
                ))
            })
        };

        match dao_request.order.dao_action {
            DaoAction::WithdrawTreasury => {
                let treasury_withdraw = TreasuryWithdraw(TreasuryWithdrawOutput {
                    token_x_asset: self.asset_x,
                    token_x_amount: dao_request.order.treasury_x_abs_delta,
                    token_y_asset: self.asset_y,
                    token_y_amount: dao_request.order.treasury_y_abs_delta,
                    // todo: fix in t2t case
                    ada_residue: 0,
                    treasury_script_hash: self.treasury_address,
                });
                Ok((
                    self.clone(),
                    OperationResultBlueprint {
                        outputs: OperationResultOutputs::multiple(requestor_output, vec![treasury_withdraw]),
                        witness_script: Some((
                            dao_validator.clone().erased(),
                            redeemer_creator(
                                self.id,
                                dao_request.order.signatures,
                                dao_request.order.additional_bytes,
                                dao_request.order.dao_action,
                            ),
                        )),
                        order_script_validator: validator,
                        strict_fee: Some(dao_ctx.execution_fee),
                    },
                ))
            }
            _ => Ok((
                self,
                OperationResultBlueprint {
                    outputs: SingleOutput(requestor_output),
                    witness_script: Some((
                        dao_validator.clone().erased(),
                        redeemer_creator(
                            self.id,
                            dao_request.order.signatures,
                            dao_request.order.additional_bytes,
                            dao_request.order.dao_action,
                        ),
                    )),
                    order_script_validator: validator,
                    strict_fee: Some(dao_ctx.execution_fee),
                },
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use cml_chain::address::Address;
    use cml_chain::transaction::TransactionOutput;
    use cml_core::serialization::Deserialize;
    use cml_crypto::ScriptHash;
    use num_rational::Ratio;
    use num_traits::ToPrimitive;
    use std::convert::identity;
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

    use crate::data::cfmm_pool::{ConstFnPool, ConstFnPoolVer};
    use crate::data::pool::PoolValidation;
    use crate::data::PoolId;
    use crate::deployment::ProtocolValidator::{
        ConstFnPoolFeeSwitch, ConstFnPoolFeeSwitchBiDirFee, ConstFnPoolFeeSwitchV2, ConstFnPoolV1,
        ConstFnPoolV2, RoyaltyPoolV1,
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
    ) -> ConstFnPool {
        return ConstFnPool {
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
            royalty_fee: Ratio::new_raw(0, 100000),
            royalty_x: TaggedAmount::new(0),
            royalty_y: TaggedAmount::new(0),
            lq_lower_bound: TaggedAmount::new(0),
            admin_address: ScriptHash::from([0; 28]),
            treasury_address: ScriptHash::from([0; 28]),
            royalty_pub_key: [0; 32],
            ver: ConstFnPoolVer::FeeSwitch,
            marginal_cost: ExUnits { mem: 100, steps: 100 },
            bounds: PoolValidation {
                min_n2t_lovelace: 10000000,
                min_t2t_lovelace: 10000000,
            },
            nonce: 0,
            stake_part_script_hash: Some(ScriptHash::from([0; 28])),
        };
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
        );
        let final_pool_distinct_swaps = distinct_swaps.iter().fold(pool_0, |acc, x| {
            acc.swap(Ask(*x)).get().fold(identity, |_| panic!())
        });
        let final_pool_aggregate_swap = pool_0
            .swap(Ask(aggregate_swap))
            .get()
            .fold(identity, |_| panic!());
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
        assert_eq!(1, 2)
    }

    #[test]
    fn treasury_x_test() {
        let pool = gen_ada_token_pool(1632109645, 1472074052, 0, 99970, 99970, 10, 11500, 2909);

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

    impl Has<DeployedScriptInfo<{ ConstFnPoolFeeSwitchBiDirFee as u8 }>> for Ctx {
        fn select<U: IsEqual<DeployedScriptInfo<{ ConstFnPoolFeeSwitchBiDirFee as u8 }>>>(
            &self,
        ) -> DeployedScriptInfo<{ ConstFnPoolFeeSwitchBiDirFee as u8 }> {
            self.scripts.const_fn_pool_fee_switch_bidir_fee
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
        let reserves_x = 2562678905; // OADA
        let reserves_y = 484492; // NIKE

        let pool = gen_ada_token_pool(
            reserves_x, reserves_y, 34991018, fee_num, fee_num, 90, 873404, 401,
        );
        let spot = pool.static_price().unwrap().to_f64().unwrap();
        let worst_price = AbsolutePrice::new(283321878, 60797).unwrap();
        let Some(AvailableLiquidity {
            input: inp,
            output: out,
        }) = pool.available_liquidity_on_side(Ask(worst_price))
        else {
            !panic!();
        };

        let Next::Succ(pool1) = pool.swap(OnSide::Ask(60797)) else {
            panic!()
        };
        let y_rec = pool.reserves_x.untag() - pool1.reserves_x.untag();

        assert_eq!(y_rec, 283321878);
        assert_eq!(inp, 60797);
        assert_eq!(out, 283321885);
    }

    #[test]
    fn available_liquidity_test() {
        let fee_num = 98500;
        let reserves_x = 1116854094529;
        let reserves_y = 4602859113047;

        let pool = gen_ada_token_pool(reserves_x, reserves_y, 0, fee_num, fee_num, 0, 0, 0);

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
