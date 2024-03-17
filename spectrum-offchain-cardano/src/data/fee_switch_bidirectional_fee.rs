use cml_chain::assets::MultiAsset;
use cml_chain::plutus::PlutusData;
use cml_chain::plutus::PlutusData::Integer;
use cml_chain::transaction::{ConwayFormatTxOut, DatumOption, TransactionOutput};
use cml_chain::utils::BigInt;
use cml_chain::Value;
use cml_multi_era::babbage::BabbageTransactionOutput;
use num_rational::Ratio;
use type_equalities::IsEqual;

use spectrum_cardano_lib::{OutputRef, TaggedAmount, TaggedAssetClass};
use spectrum_cardano_lib::plutus_data::{
    ConstrPlutusDataExtension, DatumExtension, PlutusDataExtension, RequiresRedeemer,
};
use spectrum_cardano_lib::transaction::TransactionOutputExtension;
use spectrum_cardano_lib::types::TryFromPData;
use spectrum_cardano_lib::value::ValueExtension;
use spectrum_offchain::data::{EntitySnapshot, Has, Stable, VersionUpdater};
use spectrum_offchain::ledger::{IntoLedger, TryFromLedger};

use crate::constants::{FEE_DEN, MAX_LQ_CAP};
use crate::data::{PoolId, PoolStateVer, PoolVer};
use crate::data::deposit::ClassicalOnChainDeposit;
use crate::data::limit_swap::ClassicalOnChainLimitSwap;
use crate::data::operation_output::{DepositOutput, RedeemOutput, SwapOutput};
use crate::data::order::{Base, ClassicalOrder, PoolNft, Quote};
use crate::data::pool::{ApplyOrder, ApplyOrderError, CFMMPoolAction, ImmutablePoolUtxo, Lq, Rx, Ry};
use crate::data::pool::PoolOps;
use crate::data::redeem::ClassicalOnChainRedeem;
use crate::fees::FeeExtension;
use crate::pool_math::cfmm_math::{output_amount, reward_lp, shares_amount};

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub struct FeeSwitchBidirectionalCFMMPool {
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

impl FeeSwitchBidirectionalCFMMPool {
    fn update_treasury(self, prev_datum: Option<DatumOption>) -> Option<DatumOption> {
        match prev_datum {
            None => None,
            Some(DatumOption::Hash { .. }) => None,
            Some(DatumOption::Datum { datum, .. }) => {
                let new_treasury_x = Integer(BigInt::from(self.treasury_x.untag()));
                let new_treasury_y = Integer(BigInt::from(self.treasury_y.untag()));

                let mut cpd = datum.into_constr_pd()?;

                cpd.update_field_unsafe(6, new_treasury_x);
                cpd.update_field_unsafe(7, new_treasury_y);

                Some(DatumOption::new_datum(PlutusData::ConstrPlutusData(cpd)))
            }
        }
    }
}

impl Stable for FeeSwitchBidirectionalCFMMPool {
    type StableId = PoolId;

    fn stable_id(&self) -> Self::StableId {
        self.id
    }
    fn is_quasi_permanent(&self) -> bool {
        true
    }
}

impl EntitySnapshot for FeeSwitchBidirectionalCFMMPool {
    type Version = PoolStateVer;
    fn version(&self) -> Self::Version {
        self.state_ver
    }
}

impl VersionUpdater for FeeSwitchBidirectionalCFMMPool {
    fn update_version(&mut self, new_version: Self::Version) {
        self.state_ver = new_version
    }
}

impl PoolOps for FeeSwitchBidirectionalCFMMPool {
    fn get_asset_x(&self) -> TaggedAssetClass<Rx> {
        self.asset_x
    }

    fn get_reserves_x(&self) -> TaggedAmount<Rx> {
        self.reserves_x - self.treasury_x
    }

    fn get_reserves_y(&self) -> TaggedAmount<Ry> {
        self.reserves_y - self.treasury_y
    }

    fn set_reserves_x(mut self, new_value: TaggedAmount<Rx>) -> () {
        self.reserves_x = new_value
    }

    fn set_reserves_y(mut self, new_value: TaggedAmount<Ry>) -> () {
        self.reserves_y = new_value
    }

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
            self.lp_fee_x,
            self.lp_fee_x,
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

impl ApplyOrder<ClassicalOnChainLimitSwap> for FeeSwitchBidirectionalCFMMPool {
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

impl ApplyOrder<ClassicalOnChainDeposit> for FeeSwitchBidirectionalCFMMPool {
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

impl ApplyOrder<ClassicalOnChainRedeem> for FeeSwitchBidirectionalCFMMPool {
    type Result = RedeemOutput;

    fn apply_order(
        mut self,
        ClassicalOrder { order, .. }: ClassicalOnChainRedeem,
    ) -> Result<(Self, RedeemOutput), ApplyOrderError<ClassicalOnChainRedeem>> {
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

impl Has<PoolStateVer> for FeeSwitchBidirectionalCFMMPool {
    fn get_labeled<U: IsEqual<PoolStateVer>>(&self) -> PoolStateVer {
        self.state_ver
    }
}

impl Has<PoolVer> for FeeSwitchBidirectionalCFMMPool {
    fn get_labeled<U: IsEqual<PoolVer>>(&self) -> PoolVer {
        self.ver
    }
}

impl RequiresRedeemer<CFMMPoolAction> for FeeSwitchBidirectionalCFMMPool {
    fn redeemer(action: CFMMPoolAction) -> PlutusData {
        action.to_plutus_data()
    }
}

impl TryFromLedger<BabbageTransactionOutput, OutputRef> for FeeSwitchBidirectionalCFMMPool {
    fn try_from_ledger(repr: &BabbageTransactionOutput, ctx: OutputRef) -> Option<Self> {
        if let Some(pool_ver) = PoolVer::try_from_pool_address(repr.address()) {
            let value = repr.value();
            let pd = repr.clone().into_datum()?.into_pd()?;
            let conf = FeeSwitchBidirectionalPoolConfig::try_from_pd(pd.clone())?;
            let reserves_x = TaggedAmount::new(value.amount_of(conf.asset_x.into())?);
            let reserves_y = TaggedAmount::new(value.amount_of(conf.asset_y.into())?);
            let liquidity_neg = value.amount_of(conf.asset_lq.into())?;
            let liquidity = TaggedAmount::new(MAX_LQ_CAP - liquidity_neg);
            return Some(FeeSwitchBidirectionalCFMMPool {
                id: PoolId::try_from(conf.pool_nft).ok()?,
                state_ver: PoolStateVer::from(ctx),
                reserves_x,
                reserves_y,
                liquidity,
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
            });
        }
        None
    }
}

impl IntoLedger<TransactionOutput, ImmutablePoolUtxo> for FeeSwitchBidirectionalCFMMPool {
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
        ma.set(policy_lq, name_lq.into(), MAX_LQ_CAP - self.liquidity.untag());

        TransactionOutput::new_conway_format_tx_out(ConwayFormatTxOut {
            address: immut_pool.address,
            amount: Value::new(coins, ma),
            datum_option: self.update_treasury(immut_pool.datum_option),
            script_reference: immut_pool.script_reference,
            encodings: None,
        })
    }
}

pub struct FeeSwitchBidirectionalPoolConfig {
    pool_nft: TaggedAssetClass<PoolNft>,
    asset_x: TaggedAssetClass<Rx>,
    asset_y: TaggedAssetClass<Ry>,
    asset_lq: TaggedAssetClass<Lq>,
    lp_fee_num_x: u64,
    lp_fee_num_y: u64,
    treasury_fee_num: u64,
    treasury_x: u64,
    treasury_y: u64,
    lq_lower_bound: TaggedAmount<Lq>,
}

impl TryFromPData for FeeSwitchBidirectionalPoolConfig {
    fn try_from_pd(data: PlutusData) -> Option<Self> {
        let mut cpd = data.into_constr_pd()?;
        Some(Self {
            pool_nft: TaggedAssetClass::try_from_pd(cpd.take_field(0)?)?,
            asset_x: TaggedAssetClass::try_from_pd(cpd.take_field(1)?)?,
            asset_y: TaggedAssetClass::try_from_pd(cpd.take_field(2)?)?,
            asset_lq: TaggedAssetClass::try_from_pd(cpd.take_field(3)?)?,
            lp_fee_num_x: cpd.take_field(4)?.into_u64()?,
            lp_fee_num_y: cpd.take_field(5)?.into_u64()?,
            treasury_fee_num: cpd.take_field(6)?.into_u64()?,
            treasury_x: cpd.take_field(7)?.into_u64()?,
            treasury_y: cpd.take_field(8)?.into_u64()?,
            lq_lower_bound: TaggedAmount::new(cpd.take_field(10).and_then(|pd| pd.into_u64()).unwrap_or(0)),
        })
    }
}
