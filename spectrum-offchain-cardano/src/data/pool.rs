use std::fmt::Debug;

use cml_chain::address::Address;
use cml_chain::assets::MultiAsset;
use cml_chain::builders::input_builder::SingleInputBuilder;
use cml_chain::builders::output_builder::SingleOutputBuilderResult;
use cml_chain::builders::redeemer_builder::RedeemerWitnessKey;
use cml_chain::builders::tx_builder::{
    ChangeSelectionAlgo, SignedTxBuilder, TransactionUnspentOutput, TxBuilderError,
};
use cml_chain::builders::witness_builder::{PartialPlutusWitness, PlutusScriptWitness};
use cml_chain::plutus::PlutusData::Integer;
use cml_chain::plutus::{PlutusData, RedeemerTag};
use cml_chain::transaction::{ConwayFormatTxOut, DatumOption, ScriptRef, TransactionOutput};
use cml_chain::utils::BigInt;
use cml_chain::{Coin, Value};
use cml_core::serialization::FromBytes;
use cml_multi_era::babbage::BabbageTransactionOutput;
use log::info;
use num_integer::Roots;
use num_rational::Ratio;
use type_equalities::IsEqual;

use bloom_offchain::execution_engine::liquidity_book::pool::{Pool, PoolQuality};
use bloom_offchain::execution_engine::liquidity_book::side::{Side, SideM};
use bloom_offchain::execution_engine::liquidity_book::types::AbsolutePrice;
use spectrum_cardano_lib::hash::hash_transaction_canonical;
use spectrum_cardano_lib::plutus_data::{
    ConstrPlutusDataExtension, DatumExtension, PlutusDataExtension, RequiresRedeemer,
};
use spectrum_cardano_lib::protocol_params::constant_tx_builder;
use spectrum_cardano_lib::transaction::TransactionOutputExtension;
use spectrum_cardano_lib::types::TryFromPData;
use spectrum_cardano_lib::value::ValueExtension;
use spectrum_cardano_lib::{AssetClass, OutputRef, TaggedAmount, TaggedAssetClass};
use spectrum_offchain::data::unique_entity::Predicted;
use spectrum_offchain::data::{EntitySnapshot, Has, Stable, VersionUpdater};
use spectrum_offchain::executor::RunOrderError::Fatal;
use spectrum_offchain::executor::{RunOrder, RunOrderError};
use spectrum_offchain::ledger::{IntoLedger, TryFromLedger};

use crate::constants::{
    FEE_DEN, MAX_LQ_CAP, ORDER_EXECUTION_UNITS, POOL_DESTROY_REDEEMER, POOL_EXECUTION_UNITS,
    POOL_IDX_0_DEPOSIT_REDEEMER, POOL_IDX_0_REDEEM_REDEEMER, POOL_IDX_0_SWAP_REDEEMER,
    POOL_IDX_1_DEPOSIT_REDEEMER, POOL_IDX_1_REDEEM_REDEEMER, POOL_IDX_1_SWAP_REDEEMER,
};
use crate::data::deposit::ClassicalOnChainDeposit;
use crate::data::execution_context::ExecutionContext;
use crate::data::fee_switch_bidirectional_fee::FeeSwitchBidirectionalPoolConfig;
use crate::data::fee_switch_pool::FeeSwitchPoolConfig;
use crate::data::limit_swap::ClassicalOnChainLimitSwap;
use crate::data::operation_output::{DepositOutput, RedeemOutput, SwapOutput};
use crate::data::order::{Base, ClassicalOrder, ClassicalOrderAction, PoolNft, Quote};
use crate::data::pair::order_canonical;
use crate::data::pool::ApplyOrderError::{LowBatcherFeeErr, SlippageErr};
use crate::data::redeem::ClassicalOnChainRedeem;
use crate::data::ref_scripts::RequiresRefScript;
use crate::data::{OnChain, OnChainOrderId, PoolId, PoolStateVer, PoolVer};
use crate::fees::FeeExtension;
use crate::pool_math::cfmm_math::{output_amount, reward_lp, shares_amount};

pub struct Rx;

pub struct Ry;

pub struct Lq;

pub enum ApplyOrderError<Order> {
    SlippageErr(Slippage<Order>),
    LowBatcherFeeErr(LowerBatcherFee<Order>),
}

impl<Order> ApplyOrderError<Order> {
    pub fn map<F, T1>(self, f: F) -> ApplyOrderError<T1>
    where
        F: FnOnce(Order) -> T1,
    {
        match self {
            SlippageErr(slippage) => SlippageErr(slippage.map(f)),
            LowBatcherFeeErr(low_batcher_fee) => LowBatcherFeeErr(low_batcher_fee.map(f)),
        }
    }

    pub fn slippage(
        order: Order,
        quote_amount: TaggedAmount<Quote>,
        expected_amount: TaggedAmount<Quote>,
    ) -> ApplyOrderError<Order> {
        SlippageErr(Slippage {
            order,
            quote_amount,
            expected_amount,
        })
    }

    pub fn low_batcher_fee(order: Order, batcher_fee: u64, ada_deposit: Coin) -> ApplyOrderError<Order> {
        LowBatcherFeeErr(LowerBatcherFee {
            order,
            batcher_fee,
            ada_deposit,
        })
    }
}

impl<Order> From<ApplyOrderError<Order>> for RunOrderError<Order> {
    fn from(value: ApplyOrderError<Order>) -> RunOrderError<Order> {
        match value {
            SlippageErr(slippage) => slippage.into(),
            LowBatcherFeeErr(low_batcher_fee) => low_batcher_fee.into(),
        }
    }
}

#[derive(Debug)]
pub struct Slippage<Order> {
    pub order: Order,
    pub quote_amount: TaggedAmount<Quote>,
    pub expected_amount: TaggedAmount<Quote>,
}

impl<T> Slippage<T> {
    pub fn map<F, T1>(self, f: F) -> Slippage<T1>
    where
        F: FnOnce(T) -> T1,
    {
        Slippage {
            order: f(self.order),
            quote_amount: self.quote_amount,
            expected_amount: self.expected_amount,
        }
    }
}

impl<Order> From<Slippage<Order>> for RunOrderError<Order> {
    fn from(value: Slippage<Order>) -> Self {
        RunOrderError::NonFatal("Price slippage".to_string(), value.order)
    }
}

#[derive(Debug)]
pub struct LowerBatcherFee<Order> {
    order: Order,
    batcher_fee: u64,
    ada_deposit: Coin,
}

impl<T> LowerBatcherFee<T> {
    pub fn map<F, T1>(self, f: F) -> LowerBatcherFee<T1>
    where
        F: FnOnce(T) -> T1,
    {
        LowerBatcherFee {
            order: f(self.order),
            batcher_fee: self.batcher_fee,
            ada_deposit: self.ada_deposit,
        }
    }
}

impl<Order> From<LowerBatcherFee<Order>> for RunOrderError<Order> {
    fn from(value: LowerBatcherFee<Order>) -> Self {
        RunOrderError::NonFatal(
            format!(
                "Lower batcher fee. Batcher fee {}. Ada deposit {}",
                value.batcher_fee, value.ada_deposit
            ),
            value.order,
        )
    }
}

#[derive(Debug, Copy, Clone)]
pub enum PoolInputIdx {
    Idx0,
    Idx1,
}

impl Into<u64> for PoolInputIdx {
    fn into(self) -> u64 {
        match self {
            PoolInputIdx::Idx0 => 0,
            PoolInputIdx::Idx1 => 1,
        }
    }
}

#[derive(Debug, Copy, Clone)]
pub enum OrderInputIdx {
    Idx0,
    Idx1,
}

impl Into<u64> for OrderInputIdx {
    fn into(self) -> u64 {
        match self {
            OrderInputIdx::Idx0 => 0,
            OrderInputIdx::Idx1 => 1,
        }
    }
}

pub enum CFMMPoolAction {
    Swap,
    Deposit,
    Redeem,
    Destroy,
}

impl CFMMPoolAction {
    pub fn to_plutus_data(self) -> PlutusData {
        match self {
            CFMMPoolAction::Swap => {
                PlutusData::from_bytes(hex::decode(POOL_IDX_1_SWAP_REDEEMER).unwrap()).unwrap()
            }
            CFMMPoolAction::Deposit => {
                PlutusData::from_bytes(hex::decode(POOL_IDX_0_DEPOSIT_REDEEMER).unwrap()).unwrap()
            }
            CFMMPoolAction::Redeem => {
                PlutusData::from_bytes(hex::decode(POOL_IDX_0_REDEEM_REDEEMER).unwrap()).unwrap()
            }
            CFMMPoolAction::Destroy => {
                PlutusData::from_bytes(hex::decode(POOL_DESTROY_REDEEMER).unwrap()).unwrap()
            }
        }
    }
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
    pub lp_fee_x: Ratio<u64>,
    pub lp_fee_y: Ratio<u64>,
    pub treasury_fee: Ratio<u64>,
    pub treasury_x: TaggedAmount<Rx>,
    pub treasury_y: TaggedAmount<Ry>,
    pub lq_lower_bound: TaggedAmount<Lq>,
    pub ver: PoolVer,
}

pub struct AssetDeltas {
    pub asset_to_deduct_from: AssetClass,
    pub asset_to_add_to: AssetClass,
}

impl CFMMPool {
    pub fn get_asset_deltas(&self, side: SideM) -> AssetDeltas {
        let x = self.asset_x.untag();
        let y = self.asset_y.untag();
        let [base, _] = order_canonical(x, y);
        if base == x {
            match side {
                SideM::Bid => AssetDeltas {
                    asset_to_deduct_from: x,
                    asset_to_add_to: y,
                },
                SideM::Ask => AssetDeltas {
                    asset_to_deduct_from: y,
                    asset_to_add_to: x,
                },
            }
        } else {
            match side {
                SideM::Bid => AssetDeltas {
                    asset_to_deduct_from: y,
                    asset_to_add_to: x,
                },
                SideM::Ask => AssetDeltas {
                    asset_to_deduct_from: x,
                    asset_to_add_to: y,
                },
            }
        }
    }
}

// In case of standard AMM pools we should also require
// pool idx in input set, because it affects
// final version of redeemer. Pool idx could be only 0 or 1
impl RequiresRedeemer<(CFMMPoolAction, PoolInputIdx)> for CFMMPool {
    fn redeemer(action: (CFMMPoolAction, PoolInputIdx)) -> PlutusData {
        match action {
            (CFMMPoolAction::Swap, PoolInputIdx::Idx0) => {
                PlutusData::from_bytes(hex::decode(POOL_IDX_0_SWAP_REDEEMER).unwrap()).unwrap()
            }
            (CFMMPoolAction::Swap, PoolInputIdx::Idx1) => {
                PlutusData::from_bytes(hex::decode(POOL_IDX_1_SWAP_REDEEMER).unwrap()).unwrap()
            }
            (CFMMPoolAction::Deposit, PoolInputIdx::Idx0) => {
                PlutusData::from_bytes(hex::decode(POOL_IDX_0_DEPOSIT_REDEEMER).unwrap()).unwrap()
            }
            (CFMMPoolAction::Deposit, PoolInputIdx::Idx1) => {
                PlutusData::from_bytes(hex::decode(POOL_IDX_1_DEPOSIT_REDEEMER).unwrap()).unwrap()
            }
            (CFMMPoolAction::Redeem, PoolInputIdx::Idx0) => {
                PlutusData::from_bytes(hex::decode(POOL_IDX_0_REDEEM_REDEEMER).unwrap()).unwrap()
            }
            (CFMMPoolAction::Redeem, PoolInputIdx::Idx1) => {
                PlutusData::from_bytes(hex::decode(POOL_IDX_1_REDEEM_REDEEMER).unwrap()).unwrap()
            }
            (CFMMPoolAction::Destroy, _) => {
                PlutusData::from_bytes(hex::decode(POOL_DESTROY_REDEEMER).unwrap()).unwrap()
            }
        }
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
    ) -> (TaggedAmount<Lq>, TaggedAmount<Rx>, TaggedAmount<Ry>);

    fn shares_amount(self, burned_lq: TaggedAmount<Lq>) -> (TaggedAmount<Rx>, TaggedAmount<Ry>);
}

impl AMMOps for CFMMPool {
    fn output_amount(
        &self,
        base_asset: TaggedAssetClass<Base>,
        base_amount: TaggedAmount<Base>,
    ) -> TaggedAmount<Quote> {
        output_amount(
            self.asset_x,
            self.reserves_x,
            self.reserves_y,
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
    ) -> (TaggedAmount<Lq>, TaggedAmount<Rx>, TaggedAmount<Ry>) {
        reward_lp(
            self.reserves_x,
            self.reserves_y,
            self.liquidity,
            in_x_amount,
            in_y_amount,
        )
    }

    fn shares_amount(self, burned_lq: TaggedAmount<Lq>) -> (TaggedAmount<Rx>, TaggedAmount<Ry>) {
        shares_amount(self.reserves_x, self.reserves_y, self.liquidity, burned_lq)
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
                // A user bid means that they wish to buy the base asset for the quote asset, hence
                // pool reserves of base decreases while reserves of quote increase.
                *quote_reserves += input;
                *base_reserves -= output;
                (output, self)
            }
            Side::Ask(input) => {
                // User ask is the opposite; sell the base asset for the quote asset.
                *base_reserves += input;
                *quote_reserves -= output;
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
        action.to_plutus_data()
    }
}

impl Stable for CFMMPool {
    type StableId = PoolId;
    fn stable_id(&self) -> Self::StableId {
        self.id
    }
    fn is_quasi_permanent(&self) -> bool {
        true
    }
}

impl EntitySnapshot for CFMMPool {
    type Version = PoolStateVer;
    fn version(&self) -> Self::Version {
        self.state_ver
    }
}

impl VersionUpdater for CFMMPool {
    fn update_version(&mut self, new_version: Self::Version) {
        self.state_ver = new_version
    }
}

impl TryFromLedger<BabbageTransactionOutput, OutputRef> for CFMMPool {
    fn try_from_ledger(repr: &BabbageTransactionOutput, ctx: OutputRef) -> Option<Self> {
        if let Some(pool_ver) = PoolVer::try_from_address(repr.address()) {
            let value = repr.value();
            let pd = repr.datum().clone()?.into_pd()?;
            return match pool_ver {
                PoolVer::V1 | PoolVer::V2 => {
                    let conf = LegacyCFMMPoolConfig::try_from_pd(pd.clone())?;
                    let liquidity_neg = value.amount_of(conf.asset_lq.into())?;
                    Some(CFMMPool {
                        id: PoolId::try_from(conf.pool_nft).ok()?,
                        state_ver: PoolStateVer::from(ctx),
                        reserves_x: TaggedAmount::new(value.amount_of(conf.asset_x.into())?),
                        reserves_y: TaggedAmount::new(value.amount_of(conf.asset_y.into())?),
                        liquidity: TaggedAmount::new(MAX_LQ_CAP - liquidity_neg),
                        asset_x: conf.asset_x,
                        asset_y: conf.asset_y,
                        asset_lq: conf.asset_lq,
                        lp_fee_x: Ratio::new_raw(conf.lp_fee_num, FEE_DEN),
                        lp_fee_y: Ratio::new_raw(conf.lp_fee_num, FEE_DEN),
                        treasury_fee: Ratio::new_raw(0, 1),
                        treasury_x: TaggedAmount::new(0),
                        treasury_y: TaggedAmount::new(0),
                        lq_lower_bound: conf.lq_lower_bound,
                        ver: pool_ver,
                    })
                }
                PoolVer::FeeSwitch => {
                    let conf = FeeSwitchPoolConfig::try_from_pd(pd.clone())?;
                    let liquidity_neg = value.amount_of(conf.asset_lq.into())?;
                    Some(CFMMPool {
                        id: PoolId::try_from(conf.pool_nft).ok()?,
                        state_ver: PoolStateVer::from(ctx),
                        reserves_x: TaggedAmount::new(value.amount_of(conf.asset_x.into())?)
                            - TaggedAmount::new(conf.treasury_x),
                        reserves_y: TaggedAmount::new(value.amount_of(conf.asset_y.into())?)
                            - TaggedAmount::new(conf.treasury_y),
                        liquidity: TaggedAmount::new(MAX_LQ_CAP - liquidity_neg),
                        asset_x: conf.asset_x,
                        asset_y: conf.asset_y,
                        asset_lq: conf.asset_lq,
                        lp_fee_x: Ratio::new_raw(conf.lp_fee_num, FEE_DEN),
                        lp_fee_y: Ratio::new_raw(conf.lp_fee_num, FEE_DEN),
                        treasury_fee: Ratio::new_raw(conf.treasury_fee_num, FEE_DEN),
                        treasury_x: TaggedAmount::new(conf.treasury_x),
                        treasury_y: TaggedAmount::new(conf.treasury_y),
                        lq_lower_bound: conf.lq_lower_bound,
                        ver: pool_ver,
                    })
                }
                PoolVer::FeeSwitchBiDirFee => {
                    let conf = FeeSwitchBidirectionalPoolConfig::try_from_pd(pd.clone())?;
                    let liquidity_neg = value.amount_of(conf.asset_lq.into())?;
                    Some(CFMMPool {
                        id: PoolId::try_from(conf.pool_nft).ok()?,
                        state_ver: PoolStateVer::from(ctx),
                        reserves_x: TaggedAmount::new(value.amount_of(conf.asset_x.into())?)
                            - TaggedAmount::new(conf.treasury_x),
                        reserves_y: TaggedAmount::new(value.amount_of(conf.asset_y.into())?)
                            - TaggedAmount::new(conf.treasury_y),
                        liquidity: TaggedAmount::new(MAX_LQ_CAP - liquidity_neg),
                        asset_x: conf.asset_x,
                        asset_y: conf.asset_y,
                        asset_lq: conf.asset_lq,
                        lp_fee_x: Ratio::new_raw(conf.lp_fee_num_x, FEE_DEN),
                        lp_fee_y: Ratio::new_raw(conf.lp_fee_num_y, FEE_DEN),
                        treasury_fee: Ratio::new_raw(conf.treasury_fee_num, FEE_DEN),
                        treasury_x: TaggedAmount::new(conf.treasury_x),
                        treasury_y: TaggedAmount::new(conf.treasury_y),
                        lq_lower_bound: conf.lq_lower_bound,
                        ver: pool_ver,
                    })
                }
            };
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
            let (policy_x, name_x) = self.asset_x.untag().into_token().unwrap();
            ma.set(policy_x, name_x.into(), self.reserves_x.untag());
            let (policy_y, name_y) = self.asset_y.untag().into_token().unwrap();
            ma.set(policy_y, name_y.into(), self.reserves_y.untag());
            immut_pool.value
        };
        let (policy_lq, name_lq) = self.asset_lq.untag().into_token().unwrap();
        let (nft_lq, name_nft) = self.id.into();
        ma.set(policy_lq, name_lq.into(), MAX_LQ_CAP - self.liquidity.untag());
        ma.set(nft_lq, name_nft.into(), 1);

        TransactionOutput::new_conway_format_tx_out(ConwayFormatTxOut {
            address: immut_pool.address,
            amount: Value::new(coins, ma),
            datum_option: unsafe_update_datum(&self, immut_pool.datum_option),
            script_reference: immut_pool.script_reference,
            encodings: None,
        })
    }
}

fn unsafe_update_datum(pool: &CFMMPool, prev_datum: Option<DatumOption>) -> Option<DatumOption> {
    match prev_datum {
        Some(DatumOption::Datum {
            datum,
            len_encoding,
            tag_encoding,
            datum_tag_encoding,
            datum_bytes_encoding,
        }) => match pool.ver {
            PoolVer::V1 | PoolVer::V2 => Some(DatumOption::Datum {
                datum,
                len_encoding,
                tag_encoding,
                datum_tag_encoding,
                datum_bytes_encoding,
            }),
            PoolVer::FeeSwitch | PoolVer::FeeSwitchBiDirFee => {
                let new_treasury_x = Integer(BigInt::from(pool.treasury_x.untag()));
                let new_treasury_y = Integer(BigInt::from(pool.treasury_y.untag()));

                let mut cpd = datum.into_constr_pd()?;

                cpd.update_field_unsafe(6, new_treasury_x);
                cpd.update_field_unsafe(7, new_treasury_y);

                Some(DatumOption::Datum {
                    datum: PlutusData::ConstrPlutusData(cpd),
                    len_encoding,
                    tag_encoding,
                    datum_tag_encoding,
                    datum_bytes_encoding,
                })
            }
        },
        _ => panic!("Expected inline datum"),
    }
}

pub struct LegacyCFMMPoolConfig {
    pub pool_nft: TaggedAssetClass<PoolNft>,
    pub asset_x: TaggedAssetClass<Rx>,
    pub asset_y: TaggedAssetClass<Ry>,
    pub asset_lq: TaggedAssetClass<Lq>,
    pub lp_fee_num: u64,
    pub lq_lower_bound: TaggedAmount<Lq>,
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

impl ApplyOrder<ClassicalOnChainLimitSwap> for CFMMPool {
    type Result = SwapOutput;

    fn apply_order(
        mut self,
        ClassicalOrder { id, pool_id, order }: ClassicalOnChainLimitSwap,
    ) -> Result<(Self, SwapOutput), ApplyOrderError<ClassicalOnChainLimitSwap>> {
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
            self.reserves_x = self.reserves_x - quote_amount.retag();
            self.treasury_y = self.treasury_y + TaggedAmount::new(additional_treasury_y);
            self.reserves_y = self.reserves_y + order.base_amount.retag();
        } else {
            let additional_treasury_x = (((order.base_amount.untag() as u128)
                * (*self.treasury_fee.numer() as u128))
                / (*self.treasury_fee.denom() as u128)) as u64;
            self.treasury_x = self.treasury_x + TaggedAmount::new(additional_treasury_x);
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
        Ok((self, swap_output))
    }
}

pub trait ApplyOrder<Order>: Sized {
    type Result;

    // return: new pool, order output
    fn apply_order(self, order: Order) -> Result<(Self, Self::Result), ApplyOrderError<Order>>;
}

impl ApplyOrder<ClassicalOnChainDeposit> for CFMMPool {
    type Result = DepositOutput;

    fn apply_order(
        mut self,
        ClassicalOrder { order, .. }: ClassicalOnChainDeposit,
    ) -> Result<(Self, DepositOutput), ApplyOrderError<ClassicalOnChainDeposit>> {
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
    type Result = RedeemOutput;

    fn apply_order(
        mut self,
        ClassicalOrder { order, .. }: ClassicalOnChainRedeem,
    ) -> Result<(Self, RedeemOutput), ApplyOrderError<ClassicalOnChainRedeem>> {
        let (x_amount, y_amount) = self.clone().shares_amount(order.token_lq_amount);

        self.reserves_x = self.reserves_x - x_amount;
        self.reserves_y = self.reserves_y - y_amount;
        self.liquidity = self.liquidity - order.token_lq_amount;

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

fn process_cml_step<Order>(
    step: Result<(), TxBuilderError>,
    order: OnChain<Order>,
) -> Result<(), RunOrderError<OnChain<Order>>> {
    match step {
        Ok(_) => Ok(()),
        Err(err) => {
            let error = format!("{:?}", err);
            Err(Fatal(error, order))
        }
    }
}

impl<'a, Order, Pool> RunOrder<OnChain<Order>, ExecutionContext, SignedTxBuilder> for OnChain<Pool>
where
    Pool: ApplyOrder<Order>
        + Has<PoolStateVer>
        + Has<PoolVer>
        // all AMM ops redeemers depends on action and pool/order input idx
        + RequiresRedeemer<(CFMMPoolAction, PoolInputIdx)>
        + IntoLedger<TransactionOutput, ImmutablePoolUtxo>
        + Clone
        + RequiresRefScript
        + EntitySnapshot<Version = PoolStateVer>
        + VersionUpdater,
    <Pool as ApplyOrder<Order>>::Result: IntoLedger<TransactionOutput, ExecutionContext>,
    Order: Has<OnChainOrderId>
        // all AMM ops redeemers depends on action and pool/order input idx
        + RequiresRedeemer<(ClassicalOrderAction, OrderInputIdx)>
        + RequiresRefScript
        + Clone
        + Into<CFMMPoolAction>
        + Debug,
{
    fn try_run(
        self,
        order: OnChain<Order>,
        ctx: ExecutionContext,
    ) -> Result<(SignedTxBuilder, Predicted<Self>), RunOrderError<OnChain<Order>>> {
        let pool_ref = OutputRef::from(self.value.get_labeled::<PoolStateVer>());
        let order_ref = OutputRef::from(order.value.get_labeled::<OnChainOrderId>());
        info!(target: "offchain", "Running order {} against pool {}", order_ref, pool_ref);

        let mut sorted_inputs = [pool_ref, order_ref];
        sorted_inputs.sort();

        let (pool_idx, order_idx) = match sorted_inputs {
            [pool_ref_first, _] if pool_ref_first == pool_ref => (PoolInputIdx::Idx0, OrderInputIdx::Idx1),
            _ => (PoolInputIdx::Idx1, OrderInputIdx::Idx0),
        };

        let OnChain {
            value: pool,
            source: pool_out_in,
        } = self;
        let pool_redeemer = Pool::redeemer((order.value.clone().into(), pool_idx));
        let pool_ref = OutputRef::from(pool.get_labeled::<PoolStateVer>());
        let order_ref = OutputRef::from(order.value.get_labeled::<OnChainOrderId>());
        let pool_script = PartialPlutusWitness::new(
            PlutusScriptWitness::Ref(pool_out_in.script_hash().unwrap()),
            pool_redeemer,
        );
        let immut_pool = ImmutablePoolUtxo::from(&pool_out_in);
        let pool_in = SingleInputBuilder::new(pool_ref.into(), pool_out_in.clone())
            .plutus_script_inline_datum(pool_script, Vec::new())
            .unwrap();
        let order_redeemer = Order::redeemer((ClassicalOrderAction::Apply, order_idx));
        let order_script = PartialPlutusWitness::new(
            PlutusScriptWitness::Ref(order.source.script_hash().unwrap()),
            order_redeemer,
        );
        let order_in = SingleInputBuilder::new(order_ref.into(), order.source.clone())
            .plutus_script_inline_datum(order_script, Vec::new())
            .unwrap();
        let (next_pool, user_out) = match pool.clone().apply_order(order.value.clone()) {
            Ok(res) => res,
            Err(order_error) => {
                return Err(order_error
                    .map(|value: Order| OnChain {
                        value,
                        source: order.source,
                    })
                    .into());
            }
        };
        let pool_out = next_pool.clone().into_ledger(immut_pool);

        let mut tx_builder = constant_tx_builder();

        tx_builder.add_collateral(ctx.collateral.clone().into()).unwrap();

        tx_builder.add_reference_input(order.clone().value.get_ref_script(ctx.ref_scripts.clone()));
        tx_builder.add_reference_input(pool.get_ref_script(ctx.ref_scripts.clone()));

        tx_builder.add_input(pool_in.clone()).unwrap();
        tx_builder.add_input(order_in).unwrap();

        tx_builder.set_exunits(
            RedeemerWitnessKey::new(RedeemerTag::Spend, pool_idx.clone().into()),
            POOL_EXECUTION_UNITS,
        );
        tx_builder.set_exunits(
            RedeemerWitnessKey::new(RedeemerTag::Spend, order_idx.clone().into()),
            ORDER_EXECUTION_UNITS,
        );

        process_cml_step(
            tx_builder.add_output(SingleOutputBuilderResult::new(pool_out.clone())),
            order.clone(),
        )?;

        process_cml_step(
            tx_builder.add_output(SingleOutputBuilderResult::new(user_out.into_ledger(ctx.clone()))),
            order,
        )?;

        let tx = tx_builder
            .build(ChangeSelectionAlgo::Default, &ctx.operator_addr)
            .unwrap();

        let tx_hash = hash_transaction_canonical(&tx.body());

        let mut next_pool_new = next_pool.clone();

        next_pool_new.update_version(PoolStateVer(OutputRef::new(tx_hash, pool_idx.clone().into())));

        let predicted_pool = Predicted(OnChain {
            value: next_pool_new,
            source: pool_out.clone(),
        });

        Ok((tx, predicted_pool))
    }
}

/// Reference Script Output for [CFMMPool] tagged with pool version [Ver].
#[derive(Debug, Clone)]
pub struct CFMMPoolRefScriptOutput<const VER: u8>(pub TransactionUnspentOutput);

#[cfg(test)]
pub mod tests {
    use cml_crypto::TransactionHash;
    use rand::Rng;

    use bloom_offchain::execution_engine::liquidity_book::{pool::Pool, side::Side};
    use spectrum_cardano_lib::OutputRef;
    use spectrum_offchain::ledger::TryFromLedger;
    use test_utils::pool::gen_pool_transaction_output;

    use super::CFMMPool;

    #[test]
    fn tlb_amm_pool_canonical_pair_ordering() {
        // This pool's asset order is canonical
        let pool = gen_pool(true);

        // Contains ADA
        let original_reserve_x = pool.reserves_x.untag();
        // Contains token
        let original_reserve_y = pool.reserves_y.untag();
        let ada_qty = 7000000;

        // Test Ask order (sell ADA to buy token)
        let (output_token_0, next_pool) = pool.swap(Side::Ask(ada_qty));
        let next_reserve_x = next_pool.reserves_x.untag();
        let next_reserve_y = next_pool.reserves_y.untag();
        assert_eq!(original_reserve_x, next_reserve_x - ada_qty);
        assert_eq!(original_reserve_y, next_reserve_y + output_token_0);

        // Now test Bid order (buy ADA by selling token)
        let (output_ada_1, final_pool) = next_pool.swap(Side::Bid(output_token_0));
        println!("final pool ada reserves: {}", final_pool.reserves_x.untag());
        assert_eq!(next_reserve_x, final_pool.reserves_x.untag() + output_ada_1);
        assert_eq!(next_reserve_y, final_pool.reserves_y.untag() - output_token_0);
    }

    #[test]
    fn tlb_amm_pool_non_canonical_pair_ordering() {
        // This pool's asset order is non-canonical
        let pool = gen_pool(false);

        // Contains tokens
        let original_reserve_x = pool.reserves_x.untag();
        // Contains ADA
        let original_reserve_y = pool.reserves_y.untag();
        let qty = 7000000;

        // Test Ask order (sell ADA to buy token)
        let (output_token_0, next_pool) = pool.swap(Side::Ask(qty));
        let next_reserve_x = next_pool.reserves_x.untag();
        let next_reserve_y = next_pool.reserves_y.untag();
        println!("next_x: {}, next_y: {}", next_reserve_x, next_reserve_y);
        assert_eq!(original_reserve_y, next_reserve_y - qty);
        assert_eq!(original_reserve_x, next_reserve_x + output_token_0);

        // Now test Bid order (buy ADA by selling token)
        let (output_ada_1, final_pool) = next_pool.swap(Side::Bid(output_token_0));
        assert_eq!(next_reserve_y, final_pool.reserves_y.untag() + output_ada_1);
        assert_eq!(next_reserve_x, final_pool.reserves_x.untag() - output_token_0);
    }

    fn gen_pool(ada_first: bool) -> CFMMPool {
        let (repr, _, _) = gen_pool_transaction_output(0, 101_000_000, 9_000_000, ada_first);
        let mut rng = rand::thread_rng();
        let mut bytes = [0u8; 32];
        rng.fill(&mut bytes[..]);
        let transaction_id = TransactionHash::from(bytes);
        let ctx = OutputRef::new(transaction_id, 0);
        CFMMPool::try_from_ledger(&repr, ctx).unwrap()
    }
}
