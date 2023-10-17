use cml_chain::address::Address;
use cml_chain::assets::MultiAsset;
use cml_chain::plutus::PlutusData;
use cml_chain::transaction::{ConwayFormatTxOut, DatumOption, ScriptRef, TransactionOutput};
use cml_chain::{Coin, Value};
use cml_crypto::Ed25519KeyHash;
use num_rational::Ratio;
use type_equalities::IsEqual;

use cml_core::serialization::FromBytes;

use spectrum_cardano_lib::plutus_data::{
    ConstrPlutusDataExtension, DatumExtension, PlutusDataExtension, RequiresRedeemer,
};
use spectrum_cardano_lib::transaction::TransactionOutputExtension;
use spectrum_cardano_lib::types::TryFromPData;
use spectrum_cardano_lib::value::ValueExtension;
use spectrum_cardano_lib::{OutputRef, TaggedAmount, TaggedAssetClass};
use spectrum_offchain::data::{Has, OnChainEntity};
use spectrum_offchain::executor::RunOrderError;
use spectrum_offchain::ledger::{IntoLedger, TryFromLedger};

use crate::constants::{
    CFMM_LP_FEE_DEN, MAX_LQ_CAP, POOL_DEPOSIT_REDEEMER, POOL_DESTROY_REDEEMER, POOL_REDEEM_REDEEMER,
    POOL_SWAP_REDEEMER,
};
use crate::data::batcher_output::BatcherProfit;
use crate::data::limit_swap::ClassicalOnChainLimitSwap;
use crate::data::operation_output::SwapOutput;
use crate::data::order::{Base, ClassicalOrder, PoolNft, Quote};
use crate::data::{OnChain, PoolId, PoolStateVer};

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

#[derive(Debug, Clone)]
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
        TaggedAmount::tag(quote_amount as u64)
    }
}

impl Has<PoolStateVer> for CFMMPool {
    fn get<U: IsEqual<PoolStateVer>>(&self) -> PoolStateVer {
        self.state_ver
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

impl OnChainEntity for CFMMPool {
    type TEntityId = PoolId;
    type TStateId = PoolStateVer;
    fn get_self_ref(&self) -> Self::TEntityId {
        self.id
    }
    fn get_self_state_ref(&self) -> Self::TStateId {
        self.state_ver
    }
}

impl TryFromLedger<TransactionOutput, OutputRef> for OnChain<CFMMPool> {
    fn try_from_ledger(repr: TransactionOutput, ctx: OutputRef) -> Option<Self> {
        let value = repr.amount();
        let pd = repr.clone().into_datum()?.into_pd()?;
        let conf = CFMMPoolConfig::try_from_pd(pd.clone())?;
        let reserves_x = TaggedAmount::tag(value.amount_of(conf.asset_x.into())?);
        let reserves_y = TaggedAmount::tag(value.amount_of(conf.asset_y.into())?);
        let liquidity_neg = value.amount_of(conf.asset_lq.into())?;
        let liquidity = TaggedAmount::tag(MAX_LQ_CAP - liquidity_neg);
        let pool = CFMMPool {
            id: PoolId::try_from(conf.pool_nft).ok()?,
            state_ver: PoolStateVer::from(ctx),
            reserves_x,
            reserves_y,
            liquidity,
            asset_x: conf.asset_x,
            asset_y: conf.asset_y,
            asset_lq: conf.asset_lq,
            lp_fee: Ratio::new(conf.lp_fee_num, CFMM_LP_FEE_DEN),
            lq_lower_bound: conf.lq_lower_bound,
        };
        Some(OnChain {
            value: pool,
            source: repr,
        })
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
            lq_lower_bound: TaggedAmount::tag(cpd.take_field(6).and_then(|pd| pd.into_u64()).unwrap_or(0)),
        })
    }
}

/// Defines how a particular type of swap order can be applied to the pool.
pub trait ApplySwap<Swap>: Sized {
    fn apply_swap(
        self,
        order: Swap,
        batcher_pkh: Ed25519KeyHash,
    ) -> Result<(Self, SwapOutput, BatcherProfit), Slippage<Swap>>;
}

impl ApplySwap<ClassicalOnChainLimitSwap> for CFMMPool {
    fn apply_swap(
        mut self,
        ClassicalOrder { id, pool_id, order }: ClassicalOnChainLimitSwap,
        batcher_pkh: Ed25519KeyHash,
    ) -> Result<(Self, SwapOutput, BatcherProfit), Slippage<ClassicalOnChainLimitSwap>> {
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
        let batcher_fee = order.fee.get_fee(quote_amount.untag());
        let ada_residue = order.ada_deposit - batcher_fee - 300000;
        let swap_output = SwapOutput {
            quote_asset: order.quote_asset,
            quote_amount,
            ada_residue,
            redeemer_pkh: order.redeemer_pkh,
            redeemer_stake_pkh: order.redeemer_stake_pkh,
        };
        // Prepare batcher fee.
        let batcher_profit = BatcherProfit::of(batcher_fee, batcher_pkh);
        Ok((self, swap_output, batcher_profit))
    }
}
