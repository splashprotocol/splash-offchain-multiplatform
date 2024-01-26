use std::cmp::min;

use cml_chain::address::Address;
use cml_chain::assets::MultiAsset;
use cml_chain::builders::input_builder::SingleInputBuilder;
use cml_chain::builders::output_builder::SingleOutputBuilderResult;
use cml_chain::builders::redeemer_builder::RedeemerWitnessKey;
use cml_chain::builders::tx_builder::{ChangeSelectionAlgo, SignedTxBuilder, TransactionUnspentOutput};
use cml_chain::builders::witness_builder::{PartialPlutusWitness, PlutusScriptWitness};
use cml_chain::plutus::{PlutusData, RedeemerTag};
use cml_chain::transaction::{ConwayFormatTxOut, DatumOption, ScriptRef, TransactionOutput};
use cml_chain::{Coin, Value};
use cml_core::serialization::FromBytes;
use cml_multi_era::babbage::BabbageTransactionOutput;
use log::info;
use num_integer::Roots;
use num_rational::Ratio;
use type_equalities::IsEqual;

use bloom_offchain::execution_engine::liquidity_book::pool::{Pool, PoolQuality};
use bloom_offchain::execution_engine::liquidity_book::side::Side;
use bloom_offchain::execution_engine::liquidity_book::types::AbsolutePrice;
use spectrum_cardano_lib::plutus_data::{
    ConstrPlutusDataExtension, DatumExtension, PlutusDataExtension, RequiresRedeemer,
};
use spectrum_cardano_lib::protocol_params::constant_tx_builder;
use spectrum_cardano_lib::transaction::TransactionOutputExtension;
use spectrum_cardano_lib::types::TryFromPData;
use spectrum_cardano_lib::value::ValueExtension;
use spectrum_cardano_lib::{OutputRef, TaggedAmount, TaggedAssetClass};
use spectrum_offchain::data::unique_entity::Predicted;
use spectrum_offchain::data::{EntitySnapshot, Has, Stable};
use spectrum_offchain::executor::{RunOrder, RunOrderError};
use spectrum_offchain::ledger::{IntoLedger, TryFromLedger};

use crate::constants::{
    CFMM_LP_FEE_DEN, MAX_LQ_CAP, ORDER_EXECUTION_UNITS, POOL_DEPOSIT_REDEEMER, POOL_DESTROY_REDEEMER,
    POOL_EXECUTION_UNITS, POOL_REDEEM_REDEEMER, POOL_SWAP_REDEEMER,
};
use crate::data::deposit::ClassicalOnChainDeposit;
use crate::data::execution_context::ExecutionContext;
use crate::data::limit_swap::ClassicalOnChainLimitSwap;
use crate::data::operation_output::{DepositOutput, RedeemOutput, SwapOutput};
use crate::data::order::{Base, ClassicalOrder, ClassicalOrderAction, PoolNft, Quote};
use crate::data::pair::order_canonical;
use crate::data::redeem::ClassicalOnChainRedeem;
use crate::data::ref_scripts::RequiresRefScript;
use crate::data::{OnChain, OnChainOrderId, PoolId, PoolStateVer, PoolVer};
use crate::fees::FeeExtension;

pub struct Rx;

pub struct Ry;

pub struct Lq;

#[derive(Debug)]
pub struct Slippage<Order>(Order);

impl<T> Slippage<T> {
    pub fn map<F, T1>(self, f: F) -> Slippage<T1>
    where
        F: FnOnce(T) -> T1,
    {
        Slippage(f(self.0))
    }
}

impl<Order> From<Slippage<Order>> for RunOrderError<Order> {
    fn from(value: Slippage<Order>) -> Self {
        RunOrderError::NonFatal("Price slippage".to_string(), value.0)
    }
}

pub enum CFMMPoolAction {
    Swap,
    Deposit,
    Redeem,
    Destroy,
}

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub struct CFMMPool {
    pub id: PoolId,
    pub state_ver: PoolStateVer,
    pub reserves_x: TaggedAmount<Rx>,
    pub reserves_y: TaggedAmount<Ry>,
    pub liquidity: TaggedAmount<Lq>,
    pub asset_x: TaggedAssetClass<Rx>,
    pub asset_y: TaggedAssetClass<Ry>,
    pub asset_lq: TaggedAssetClass<Lq>,
    pub lp_fee: Ratio<u64>,
    pub lq_lower_bound: TaggedAmount<Lq>,
    pub ver: PoolVer,
}

impl CFMMPool {
    pub fn output_amount(
        &self,
        base_asset: TaggedAssetClass<Base>,
        base_amount: TaggedAmount<Base>,
    ) -> TaggedAmount<Quote> {
        let quote_amount = if base_asset.untag() == self.asset_x.untag() {
            (self.reserves_y.untag() as u128) * (base_amount.untag() as u128) * (*self.lp_fee.numer() as u128)
                / ((self.reserves_x.untag() as u128) * (*self.lp_fee.denom() as u128)
                    + (base_amount.untag() as u128) * (*self.lp_fee.numer() as u128))
        } else {
            (self.reserves_x.untag() as u128) * (base_amount.untag() as u128) * (*self.lp_fee.numer() as u128)
                / ((self.reserves_y.untag() as u128) * (*self.lp_fee.denom() as u128)
                    + (base_amount.untag() as u128) * (*self.lp_fee.numer() as u128))
        };
        TaggedAmount::new(quote_amount as u64)
    }

    pub fn reward_lp(
        &self,
        in_x_amount: u64,
        in_y_amount: u64,
    ) -> (TaggedAmount<Lq>, TaggedAmount<Rx>, TaggedAmount<Ry>) {
        let min_by_x =
            (in_x_amount as u128) * (self.liquidity.untag() as u128) / (self.reserves_x.untag() as u128);
        let min_by_y =
            (in_y_amount as u128) * (self.liquidity.untag() as u128) / (self.reserves_y.untag() as u128);
        let (change_by_x, change_by_y): (u64, u64) = if min_by_x == min_by_y {
            (0, 0)
        } else {
            if min_by_x < min_by_y {
                (
                    0,
                    ((min_by_y - min_by_x) * (self.reserves_y.untag() as u128)
                        / (self.liquidity.untag() as u128)) as u64,
                )
            } else {
                (
                    ((min_by_x - min_by_y) * (self.reserves_x.untag() as u128)
                        / (self.liquidity.untag() as u128)) as u64,
                    0,
                )
            }
        };
        let unlocked_lq = min(min_by_x, min_by_y) as u64;
        (
            TaggedAmount::new(unlocked_lq),
            TaggedAmount::new(change_by_x),
            TaggedAmount::new(change_by_y),
        )
    }

    pub fn shares_amount(self, burned_lq: TaggedAmount<Lq>) -> (TaggedAmount<Rx>, TaggedAmount<Ry>) {
        let x_amount = (burned_lq.untag() as u128) * (self.reserves_x.untag() as u128)
            / (self.liquidity.untag() as u128);
        let y_amount = (burned_lq.untag() as u128) * (self.reserves_y.untag() as u128)
            / (self.liquidity.untag() as u128);

        (
            TaggedAmount::new(x_amount as u64),
            TaggedAmount::new(y_amount as u64),
        )
    }
}

impl Pool for CFMMPool {
    fn static_price(&self) -> AbsolutePrice {
        let x = self.asset_x.untag();
        let y = self.asset_y.untag();
        let [base, _] = order_canonical(x, y);
        if x == base {
            AbsolutePrice::new(self.reserves_y.untag(), self.reserves_x.untag())
        } else {
            AbsolutePrice::new(self.reserves_x.untag(), self.reserves_y.untag())
        }
    }

    fn real_price(&self, input: Side<u64>) -> AbsolutePrice {
        let x = self.asset_x.untag();
        let y = self.asset_y.untag();
        let [base, quote] = order_canonical(x, y);
        let (base, quote) = match input {
            Side::Bid(input) => (
                self.output_amount(TaggedAssetClass::new(quote), TaggedAmount::new(input))
                    .untag(),
                input,
            ),
            Side::Ask(input) => (
                input,
                self.output_amount(TaggedAssetClass::new(base), TaggedAmount::new(input))
                    .untag(),
            ),
        };
        AbsolutePrice::new(quote, base)
    }

    fn swap(mut self, input: Side<u64>) -> (u64, Self) {
        let x = self.asset_x.untag();
        let y = self.asset_y.untag();
        let [base, quote] = order_canonical(x, y);
        let output = match input {
            Side::Bid(input) => self
                .output_amount(TaggedAssetClass::new(quote), TaggedAmount::new(input))
                .untag(),
            Side::Ask(input) => self
                .output_amount(TaggedAssetClass::new(base), TaggedAmount::new(input))
                .untag(),
        };
        let (base_reserves, quote_reserves) = if x == base {
            (self.reserves_x.as_mut(), self.reserves_y.as_mut())
        } else {
            (self.reserves_y.as_mut(), self.reserves_x.as_mut())
        };
        match input {
            Side::Bid(input) => {
                *quote_reserves -= input;
                *base_reserves += output;
                (output, self)
            }
            Side::Ask(input) => {
                *base_reserves -= input;
                *quote_reserves += output;
                (output, self)
            }
        }
    }

    fn quality(&self) -> PoolQuality {
        let lq = (self.reserves_x.untag() * self.reserves_y.untag()).sqrt();
        PoolQuality(self.static_price(), lq)
    }
}

impl Has<PoolStateVer> for CFMMPool {
    fn get_labeled<U: IsEqual<PoolStateVer>>(&self) -> PoolStateVer {
        self.state_ver
    }
}

impl Has<PoolVer> for CFMMPool {
    fn get_labeled<U: IsEqual<PoolVer>>(&self) -> PoolVer {
        self.ver
    }
}

impl RequiresRedeemer<CFMMPoolAction> for CFMMPool {
    fn redeemer(action: CFMMPoolAction) -> PlutusData {
        match action {
            CFMMPoolAction::Swap => PlutusData::from_bytes(hex::decode(POOL_SWAP_REDEEMER).unwrap()).unwrap(),
            CFMMPoolAction::Deposit => {
                PlutusData::from_bytes(hex::decode(POOL_DEPOSIT_REDEEMER).unwrap()).unwrap()
            }
            CFMMPoolAction::Redeem => {
                PlutusData::from_bytes(hex::decode(POOL_REDEEM_REDEEMER).unwrap()).unwrap()
            }
            CFMMPoolAction::Destroy => {
                PlutusData::from_bytes(hex::decode(POOL_DESTROY_REDEEMER).unwrap()).unwrap()
            }
        }
    }
}

impl Stable for CFMMPool {
    type StableId = PoolId;
    fn stable_id(&self) -> Self::StableId {
        self.id
    }
}

impl EntitySnapshot for CFMMPool {
    type Version = PoolStateVer;
    fn version(&self) -> Self::Version {
        self.state_ver
    }
}

impl<C> TryFromLedger<BabbageTransactionOutput, C> for CFMMPool
where
    C: Has<OutputRef>,
{
    fn try_from_ledger(repr: &BabbageTransactionOutput, ctx: C) -> Option<Self> {
        if let Some(pool_ver) = PoolVer::try_from_pool_address(repr.address()) {
            let value = repr.value();
            let pd = repr.datum().clone()?.into_pd()?;
            let conf = CFMMPoolConfig::try_from_pd(pd.clone())?;
            let reserves_x = TaggedAmount::new(value.amount_of(conf.asset_x.into())?);
            let reserves_y = TaggedAmount::new(value.amount_of(conf.asset_y.into())?);
            let liquidity_neg = value.amount_of(conf.asset_lq.into())?;
            let liquidity = TaggedAmount::new(MAX_LQ_CAP - liquidity_neg);
            return Some(CFMMPool {
                id: PoolId::try_from(conf.pool_nft).ok()?,
                state_ver: ctx.get().into(),
                reserves_x,
                reserves_y,
                liquidity,
                asset_x: conf.asset_x,
                asset_y: conf.asset_y,
                asset_lq: conf.asset_lq,
                lp_fee: Ratio::new(conf.lp_fee_num, CFMM_LP_FEE_DEN),
                lq_lower_bound: conf.lq_lower_bound,
                ver: pool_ver,
            });
        }
        None
    }
}

pub struct ImmutablePoolUtxo {
    pub address: Address,
    pub value: Coin,
    pub datum_option: Option<DatumOption>,
    pub script_reference: Option<ScriptRef>,
}

impl From<&TransactionOutput> for ImmutablePoolUtxo {
    fn from(out: &TransactionOutput) -> Self {
        Self {
            address: out.address().clone(),
            value: out.amount().coin,
            datum_option: out.datum(),
            script_reference: out.script_ref().cloned(),
        }
    }
}

impl IntoLedger<TransactionOutput, ImmutablePoolUtxo> for CFMMPool {
    fn into_ledger(self, immut_pool: ImmutablePoolUtxo) -> TransactionOutput {
        let mut ma = MultiAsset::new();
        let coins = if self.asset_x.is_native() {
            let (policy, name) = self.asset_y.untag().into_token().unwrap();
            ma.set(policy, name.into(), self.reserves_y.untag());
            self.reserves_x.untag()
        } else if self.asset_y.is_native() {
            let (policy, name) = self.asset_x.untag().into_token().unwrap();
            ma.set(policy, name.into(), self.reserves_x.untag());
            self.reserves_y.untag()
        } else {
            let (policy_x, name_x) = self.asset_y.untag().into_token().unwrap();
            ma.set(policy_x, name_x.into(), self.reserves_y.untag());
            let (policy_y, name_y) = self.asset_y.untag().into_token().unwrap();
            ma.set(policy_y, name_y.into(), self.reserves_y.untag());
            immut_pool.value
        };
        let (policy_lq, name_lq) = self.asset_lq.untag().into_token().unwrap();
        ma.set(policy_lq, name_lq.into(), self.liquidity.untag());

        TransactionOutput::new_conway_format_tx_out(ConwayFormatTxOut {
            address: immut_pool.address,
            amount: Value::new(coins, ma),
            datum_option: immut_pool.datum_option,
            script_reference: immut_pool.script_reference,
            encodings: None,
        })
    }
}

pub struct CFMMPoolConfig {
    pool_nft: TaggedAssetClass<PoolNft>,
    asset_x: TaggedAssetClass<Rx>,
    asset_y: TaggedAssetClass<Ry>,
    asset_lq: TaggedAssetClass<Lq>,
    lp_fee_num: u64,
    lq_lower_bound: TaggedAmount<Lq>,
}

impl TryFromPData for CFMMPoolConfig {
    fn try_from_pd(data: PlutusData) -> Option<Self> {
        let mut cpd = data.into_constr_pd()?;
        Some(Self {
            pool_nft: TaggedAssetClass::try_from_pd(cpd.take_field(0)?)?,
            asset_x: TaggedAssetClass::try_from_pd(cpd.take_field(1)?)?,
            asset_y: TaggedAssetClass::try_from_pd(cpd.take_field(2)?)?,
            asset_lq: TaggedAssetClass::try_from_pd(cpd.take_field(3)?)?,
            lp_fee_num: cpd.take_field(4)?.into_u64()?,
            lq_lower_bound: TaggedAmount::new(cpd.take_field(6).and_then(|pd| pd.into_u64()).unwrap_or(0)),
        })
    }
}

/// Defines how a particular type of swap order can be applied to the pool.
pub trait ApplySwap<Swap>: Sized {
    fn apply_swap(self, order: Swap) -> Result<(Self, SwapOutput), Slippage<Swap>>;
}

impl ApplyOrder<ClassicalOnChainLimitSwap> for CFMMPool {
    type OrderApplicationResult = SwapOutput;

    fn apply_order(
        mut self,
        ClassicalOrder { id, pool_id, order }: ClassicalOnChainLimitSwap,
    ) -> Result<(Self, SwapOutput), Slippage<ClassicalOnChainLimitSwap>> {
        let quote_amount = self.output_amount(order.base_asset, order.base_amount);
        if quote_amount < order.min_expected_quote_amount {
            return Err(Slippage(ClassicalOrder { id, pool_id, order }));
        }
        // Adjust pool value.
        if order.quote_asset.untag() == self.asset_x.untag() {
            self.reserves_x = self.reserves_x - quote_amount.retag();
            self.reserves_y = self.reserves_y + order.base_amount.retag();
        } else {
            self.reserves_y = self.reserves_y - quote_amount.retag();
            self.reserves_x = self.reserves_x + order.base_amount.retag();
        }
        // Prepare user output.
        let batcher_fee = order.fee.value().linear_fee(quote_amount.untag());
        let ada_residue = order.ada_deposit - batcher_fee;
        let swap_output = SwapOutput {
            quote_asset: order.quote_asset,
            quote_amount,
            ada_residue,
            redeemer_pkh: order.redeemer_pkh,
            redeemer_stake_pkh: order.redeemer_stake_pkh,
        };
        // Prepare batcher fee.
        Ok((self, swap_output))
    }
}

pub trait ApplyOrder<Order>: Sized {
    type OrderApplicationResult;

    // return: new pool, order output
    fn apply_order(self, order: Order) -> Result<(Self, Self::OrderApplicationResult), Slippage<Order>>;
}

impl ApplyOrder<ClassicalOnChainDeposit> for CFMMPool {
    type OrderApplicationResult = DepositOutput;

    fn apply_order(
        mut self,
        ClassicalOrder { order, .. }: ClassicalOnChainDeposit,
    ) -> Result<(Self, DepositOutput), Slippage<ClassicalOnChainDeposit>> {
        let net_x = if order.token_x.is_native() {
            order.token_x_amount.untag() - order.ex_fee - order.collateral_ada
        } else {
            order.token_x_amount.untag()
        };

        let net_y = if order.token_y.is_native() {
            order.token_y_amount.untag() - order.ex_fee - order.collateral_ada
        } else {
            order.token_y_amount.untag()
        };

        let (unlocked_lq, change_x, change_y) = self.reward_lp(net_x, net_y);

        self.reserves_x = self.reserves_x + TaggedAmount::new(net_x) - change_x;
        self.reserves_y = self.reserves_y + TaggedAmount::new(net_y) - change_y;
        self.liquidity = self.liquidity + unlocked_lq;

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
}

impl ApplyOrder<ClassicalOnChainRedeem> for CFMMPool {
    type OrderApplicationResult = RedeemOutput;

    fn apply_order(
        mut self,
        ClassicalOrder { order, .. }: ClassicalOnChainRedeem,
    ) -> Result<(Self, RedeemOutput), Slippage<ClassicalOnChainRedeem>> {
        let (x_amount, y_amount) = self.clone().shares_amount(order.token_lq_amount);

        self.reserves_x = self.reserves_x - x_amount;
        self.reserves_y = self.reserves_y - y_amount;
        self.liquidity = self.liquidity + order.token_lq_amount;

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
}

impl<'a, Order, Pool> RunOrder<OnChain<Order>, ExecutionContext, SignedTxBuilder> for OnChain<Pool>
where
    Pool: ApplyOrder<Order>
        + Has<PoolStateVer>
        + Has<PoolVer>
        + RequiresRedeemer<CFMMPoolAction>
        + IntoLedger<TransactionOutput, ImmutablePoolUtxo>
        + Clone
        + RequiresRefScript,
    <Pool as ApplyOrder<Order>>::OrderApplicationResult: IntoLedger<TransactionOutput, ()>,
    Order: Has<OnChainOrderId>
        + RequiresRedeemer<ClassicalOrderAction>
        + RequiresRefScript
        + Clone
        + Into<CFMMPoolAction>,
{
    fn try_run(
        self,
        OnChain {
            value: order,
            source: order_out_in,
        }: OnChain<Order>,
        ctx: ExecutionContext,
    ) -> Result<(SignedTxBuilder, Predicted<Self>), RunOrderError<OnChain<Order>>> {
        let OnChain {
            value: pool,
            source: pool_out_in,
        } = self;
        let pool_ref = OutputRef::from(pool.get_labeled::<PoolStateVer>());
        let order_ref = OutputRef::from(order.get_labeled::<OnChainOrderId>());
        info!(target: "offchain", "Running order {} against pool {}", order_ref, pool_ref);
        let (next_pool, user_out) = match pool.clone().apply_order(order.clone()) {
            Ok(res) => res,
            Err(slippage) => {
                return Err(slippage
                    .map(|value: Order| OnChain {
                        value,
                        source: order_out_in,
                    })
                    .into());
            }
        };
        let pool_redeemer = Pool::redeemer(order.clone().into());
        let pool_script = PartialPlutusWitness::new(
            PlutusScriptWitness::Ref(pool_out_in.script_hash().unwrap()),
            pool_redeemer,
        );
        let immut_pool = ImmutablePoolUtxo::from(&pool_out_in);
        let pool_in = SingleInputBuilder::new(pool_ref.into(), pool_out_in)
            .plutus_script_inline_datum(pool_script, Vec::new())
            .unwrap();
        let order_redeemer = Order::redeemer(ClassicalOrderAction::Apply);
        let order_script = PartialPlutusWitness::new(
            PlutusScriptWitness::Ref(order_out_in.script_hash().unwrap()),
            order_redeemer,
        );
        let order_in = SingleInputBuilder::new(order_ref.into(), order_out_in)
            .plutus_script_inline_datum(order_script, Vec::new())
            .unwrap();
        let pool_out = next_pool.clone().into_ledger(immut_pool);
        let predicted_pool = Predicted(OnChain {
            value: next_pool,
            source: pool_out.clone(),
        });

        let mut tx_builder = constant_tx_builder();

        tx_builder.add_collateral(ctx.collateral.into()).unwrap();

        tx_builder.add_reference_input(order.clone().get_ref_script(ctx.ref_scripts.clone()));
        tx_builder.add_reference_input(pool.get_ref_script(ctx.ref_scripts.clone()));

        tx_builder.add_input(pool_in.clone()).unwrap();
        tx_builder.add_input(order_in).unwrap();

        tx_builder.set_exunits(
            RedeemerWitnessKey::new(RedeemerTag::Spend, 0),
            POOL_EXECUTION_UNITS,
        );

        tx_builder.set_exunits(
            RedeemerWitnessKey::new(RedeemerTag::Spend, 1),
            ORDER_EXECUTION_UNITS,
        );

        tx_builder
            .add_output(SingleOutputBuilderResult::new(pool_out))
            .unwrap();
        tx_builder
            .add_output(SingleOutputBuilderResult::new(user_out.into_ledger(())))
            .unwrap();

        let tx = tx_builder
            .build(ChangeSelectionAlgo::Default, &ctx.operator_addr)
            .unwrap();

        Ok((tx, predicted_pool))
    }
}

/// Reference Script Output for [CFMMPool] tagged with pool version [Ver].
#[derive(Debug, Clone)]
pub struct CFMMPoolRefScriptOutput<const VER: u8>(pub TransactionUnspentOutput);
