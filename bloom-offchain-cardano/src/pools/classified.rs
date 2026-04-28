use std::fmt::{Display, Formatter};

use bloom_offchain::execution_engine::liquidity_book::core::{MakeInProgress, Next, TakeInProgress, Trans};
use bloom_offchain::execution_engine::liquidity_book::market_maker::{
    default_swap_with_taker, AbsoluteReserves, AvailableLiquidity, MakerBehavior, MarketMaker, PoolQuality,
    SpotPrice,
};
use bloom_offchain::execution_engine::liquidity_book::market_taker::{MarketTaker, TakerBehaviour};
use bloom_offchain::execution_engine::liquidity_book::side::OnSide;
use bloom_offchain::execution_engine::liquidity_book::types::{AbsolutePrice, Lovelace};
use cml_chain::transaction::TransactionOutput;
use spectrum_cardano_lib::AssetClass;
use spectrum_cardano_lib::Token;
use spectrum_offchain::domain::{Has, Stable, Tradable};
use spectrum_offchain::ledger::TryFromLedger;
use spectrum_offchain_cardano::data::pool::AnyPool;
use spectrum_offchain_cardano::data::pool::PoolValidation;
use spectrum_offchain_cardano::deployment::DeployedScriptInfo;
use spectrum_offchain_cardano::deployment::ProtocolValidator::{
    BalanceFnPoolV1, BalanceFnPoolV2, ConstFnPoolFeeSwitch, ConstFnPoolFeeSwitchBiDirFee,
    ConstFnPoolFeeSwitchV2, ConstFnPoolV1, ConstFnPoolV2, RoyaltyPoolV1, RoyaltyPoolV1LedgerFixed,
    RoyaltyPoolV2, StableFnPoolT2T,
};

use crate::graduation::{GraduatedPoolFeeConfig, GraduatedSplashPoolStore, PoolOrigin};

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub struct ClassifiedPool {
    pub inner: AnyPool,
    pub origin: PoolOrigin,
    pub fee_config: GraduatedPoolFeeConfig,
    pub pending_operator_fee: Lovelace,
}

impl ClassifiedPool {
    fn fee_applicable<Taker>(&self, taker: &Taker) -> bool
    where
        Taker: TakerBehaviour,
    {
        self.origin == PoolOrigin::SnekGraduated
            && self.fee_config.enabled
            && taker.graduated_splash_fee_eligible()
    }

    fn trade_assets(&self, input: OnSide<u64>) -> (AssetClass, AssetClass) {
        let (base, quote) = self.pair_id().assets();
        match input {
            OnSide::Ask(_) => (base, quote),
            OnSide::Bid(_) => (quote, base),
        }
    }

    fn with_inner(self, inner: AnyPool, pending_operator_fee: Lovelace) -> Self {
        Self {
            inner,
            pending_operator_fee,
            ..self
        }
    }
}

impl Display for ClassifiedPool {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(format!("ClassifiedPool(origin={:?}, {})", self.origin, self.inner).as_str())
    }
}

impl Stable for ClassifiedPool {
    type StableId = Token;

    fn stable_id(&self) -> Self::StableId {
        self.inner.stable_id()
    }

    fn is_quasi_permanent(&self) -> bool {
        self.inner.is_quasi_permanent()
    }
}

impl Tradable for ClassifiedPool {
    type PairId = <AnyPool as Tradable>::PairId;

    fn pair_id(&self) -> Self::PairId {
        self.inner.pair_id()
    }
}

impl MakerBehavior for ClassifiedPool {
    fn swap(self, input: OnSide<u64>) -> Next<Self, void::Void> {
        self.inner.swap(input).map_succ(|inner| self.with_inner(inner, 0))
    }

    fn preserve_preview_metadata(self, previewed: Self, mut rebalanced: Self) -> Self {
        rebalanced.pending_operator_fee = previewed.pending_operator_fee;
        rebalanced
    }

    fn swap_with_taker<Taker>(
        self,
        target_taker: Taker,
        input: OnSide<u64>,
    ) -> (TakeInProgress<Taker>, MakeInProgress<Self>)
    where
        Taker: MarketTaker + TakerBehaviour + Copy,
        Self: MarketMaker + MakerBehavior + Copy,
    {
        if !self.fee_applicable(&target_taker) {
            return default_swap_with_taker(target_taker, self, input);
        }

        let (input_asset, output_asset) = self.trade_assets(input);
        let gross_input = input.unwrap();
        if input_asset == AssetClass::Native {
            let operator_fee = self.fee_config.fee(gross_input);
            let Some(net_input) = gross_input.checked_sub(operator_fee) else {
                return default_swap_with_taker(target_taker, self, input);
            };
            if net_input == 0 {
                return default_swap_with_taker(target_taker, self, input);
            }
            let maker_input = input.map(|_| net_input);
            let next_inner = self.inner.swap(maker_input);
            let make = Trans::new(
                self,
                next_inner.map_succ(|inner| self.with_inner(inner, operator_fee)),
            );
            let trade_output = make.loss().map(|val| val.unwrap()).unwrap_or(0);
            let next_taker = target_taker.with_applied_trade(gross_input, trade_output);
            return (Trans::new(target_taker, next_taker), make);
        }

        if output_asset == AssetClass::Native {
            let next_inner = self.inner.swap(input);
            let gross_output = Trans::new(self.inner, next_inner)
                .loss()
                .map(|val| val.unwrap())
                .unwrap_or(0);
            let operator_fee = self.fee_config.fee(gross_output);
            let Some(net_output) = gross_output.checked_sub(operator_fee) else {
                return default_swap_with_taker(target_taker, self, input);
            };
            let make = Trans::new(
                self,
                next_inner.map_succ(|inner| self.with_inner(inner, operator_fee)),
            );
            let next_taker = target_taker.with_applied_trade(gross_input, net_output);
            return (Trans::new(target_taker, next_taker), make);
        }

        default_swap_with_taker(target_taker, self, input)
    }
}

impl MarketMaker for ClassifiedPool {
    type U = <AnyPool as MarketMaker>::U;

    fn static_price(&self) -> SpotPrice {
        self.inner.static_price()
    }

    fn real_price(&self, input: OnSide<u64>) -> Option<AbsolutePrice> {
        self.inner.real_price(input)
    }

    fn effective_price<Taker>(&self, taker: &Taker, input: OnSide<u64>) -> Option<AbsolutePrice>
    where
        Taker: MarketTaker + TakerBehaviour + Copy,
    {
        if !self.fee_applicable(taker) {
            return self.inner.real_price(input);
        }
        let (input_asset, output_asset) = self.trade_assets(input);
        let gross_input = input.unwrap();
        if input_asset == AssetClass::Native {
            let net_input = gross_input.checked_sub(self.fee_config.fee(gross_input))?;
            let output = self.inner.estimated_trade(input.map(|_| net_input))?.output;
            return match input {
                OnSide::Ask(_) => AbsolutePrice::new(output, gross_input),
                OnSide::Bid(_) => AbsolutePrice::new(gross_input, output),
            };
        }
        if output_asset == AssetClass::Native {
            let estimated = self.inner.estimated_trade(input)?;
            let net_output = estimated
                .output
                .checked_sub(self.fee_config.fee(estimated.output))?;
            return match input {
                OnSide::Ask(_) => AbsolutePrice::new(net_output, gross_input),
                OnSide::Bid(_) => AbsolutePrice::new(gross_input, net_output),
            };
        }
        self.inner.real_price(input)
    }

    fn quality(&self) -> PoolQuality {
        self.inner.quality()
    }

    fn marginal_cost_hint(&self) -> Self::U {
        self.inner.marginal_cost_hint()
    }

    fn liquidity(&self) -> AbsoluteReserves {
        self.inner.liquidity()
    }

    fn available_liquidity_on_side(&self, worst_price: OnSide<AbsolutePrice>) -> Option<AvailableLiquidity> {
        self.inner.available_liquidity_on_side(worst_price)
    }

    fn estimated_trade(&self, input: OnSide<u64>) -> Option<AvailableLiquidity> {
        self.inner.estimated_trade(input)
    }

    fn is_active(&self) -> bool {
        self.inner.is_active()
    }
}

impl<C> TryFromLedger<TransactionOutput, C> for ClassifiedPool
where
    C: Has<DeployedScriptInfo<{ ConstFnPoolV1 as u8 }>>
        + Has<DeployedScriptInfo<{ ConstFnPoolV2 as u8 }>>
        + Has<DeployedScriptInfo<{ ConstFnPoolFeeSwitch as u8 }>>
        + Has<DeployedScriptInfo<{ ConstFnPoolFeeSwitchV2 as u8 }>>
        + Has<DeployedScriptInfo<{ ConstFnPoolFeeSwitchBiDirFee as u8 }>>
        + Has<DeployedScriptInfo<{ BalanceFnPoolV1 as u8 }>>
        + Has<DeployedScriptInfo<{ BalanceFnPoolV2 as u8 }>>
        + Has<DeployedScriptInfo<{ StableFnPoolT2T as u8 }>>
        + Has<DeployedScriptInfo<{ RoyaltyPoolV1 as u8 }>>
        + Has<DeployedScriptInfo<{ RoyaltyPoolV1LedgerFixed as u8 }>>
        + Has<DeployedScriptInfo<{ RoyaltyPoolV2 as u8 }>>
        + Has<PoolValidation>
        + Has<GraduatedSplashPoolStore>
        + Has<GraduatedPoolFeeConfig>,
{
    fn try_from_ledger(repr: &TransactionOutput, ctx: &C) -> Option<Self> {
        let inner = AnyPool::try_from_ledger(repr, ctx)?;
        let origin = if ctx
            .select::<GraduatedSplashPoolStore>()
            .contains(inner.stable_id())
        {
            PoolOrigin::SnekGraduated
        } else {
            PoolOrigin::Direct
        };
        Some(Self {
            inner,
            origin,
            fee_config: ctx.select::<GraduatedPoolFeeConfig>(),
            pending_operator_fee: 0,
        })
    }
}
