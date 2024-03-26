use cml_chain::address::Address;
use cml_chain::assets::MultiAsset;
use cml_chain::certs::StakeCredential;
use std::fmt::Debug;

use cml_chain::plutus::{ConstrPlutusData, PlutusData};
use cml_chain::transaction::{ConwayFormatTxOut, DatumOption, TransactionOutput};
use cml_chain::utils::BigInteger;
use cml_chain::Value;

use cml_multi_era::babbage::BabbageTransactionOutput;
use num_integer::Roots;
use num_rational::Ratio;
use type_equalities::IsEqual;

use bloom_offchain::execution_engine::liquidity_book::pool::{Pool, PoolQuality};
use bloom_offchain::execution_engine::liquidity_book::side::{Side, SideM};
use bloom_offchain::execution_engine::liquidity_book::types::AbsolutePrice;
use spectrum_cardano_lib::plutus_data::{
    ConstrPlutusDataExtension, DatumExtension, IntoPlutusData, PlutusDataExtension,
};
use spectrum_cardano_lib::transaction::TransactionOutputExtension;
use spectrum_cardano_lib::types::TryFromPData;
use spectrum_cardano_lib::value::ValueExtension;
use spectrum_cardano_lib::{TaggedAmount, TaggedAssetClass};
use spectrum_offchain::data::{Has, Stable};
use spectrum_offchain::ledger::{IntoLedger, TryFromLedger};

use crate::constants::{FEE_DEN, MAX_LQ_CAP};

use crate::data::deposit::ClassicalOnChainDeposit;

use crate::data::fee_switch_bidirectional_fee::FeeSwitchBidirectionalPoolConfig;

use crate::data::limit_swap::ClassicalOnChainLimitSwap;
use crate::data::operation_output::{DepositOutput, RedeemOutput, SwapOutput};
use crate::data::order::{Base, ClassicalOrder, PoolNft, Quote};
use crate::data::pair::order_canonical;

use crate::data::pool::{ApplyOrder, ApplyOrderError, AssetDeltas, ImmutablePoolUtxo, Lq, Rx, Ry};
use crate::data::redeem::ClassicalOnChainRedeem;

use crate::data::fee_switch_pool::FeeSwitchPoolConfig;
use crate::data::PoolId;
use crate::deployment::ProtocolValidator::{
    ConstFnPoolFeeSwitch, ConstFnPoolFeeSwitchBiDirFee, ConstFnPoolV1, ConstFnPoolV2,
};
use crate::deployment::{DeployedScriptHash, DeployedValidator, DeployedValidatorErased, RequiresValidator};
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

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub enum ConstFnPoolVer {
    V1,
    V2,
    FeeSwitch,
    FeeSwitchBiDirFee,
}

impl ConstFnPoolVer {
    pub fn try_from_address<Ctx>(pool_addr: &Address, ctx: &Ctx) -> Option<ConstFnPoolVer>
    where
        Ctx: Has<DeployedScriptHash<{ ConstFnPoolV1 as u8 }>>
            + Has<DeployedScriptHash<{ ConstFnPoolV2 as u8 }>>
            + Has<DeployedScriptHash<{ ConstFnPoolFeeSwitch as u8 }>>
            + Has<DeployedScriptHash<{ ConstFnPoolFeeSwitchBiDirFee as u8 }>>,
    {
        let maybe_hash = pool_addr.payment_cred().and_then(|c| match c {
            StakeCredential::PubKey { .. } => None,
            StakeCredential::Script { hash, .. } => Some(hash),
        });
        if let Some(this_hash) = maybe_hash {
            if ctx
                .select::<DeployedScriptHash<{ ConstFnPoolV1 as u8 }>>()
                .unwrap()
                == *this_hash
            {
                return Some(ConstFnPoolVer::V1);
            } else if ctx
                .select::<DeployedScriptHash<{ ConstFnPoolV2 as u8 }>>()
                .unwrap()
                == *this_hash
            {
                return Some(ConstFnPoolVer::V2);
            } else if ctx
                .select::<DeployedScriptHash<{ ConstFnPoolFeeSwitch as u8 }>>()
                .unwrap()
                == *this_hash
            {
                return Some(ConstFnPoolVer::FeeSwitch);
            } else if ctx
                .select::<DeployedScriptHash<{ ConstFnPoolFeeSwitchBiDirFee as u8 }>>()
                .unwrap()
                == *this_hash
            {
                return Some(ConstFnPoolVer::FeeSwitchBiDirFee);
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
    pub lq_lower_bound: TaggedAmount<Lq>,
    pub ver: ConstFnPoolVer,
}

impl ConstFnPool {
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
    ) -> (TaggedAmount<Lq>, TaggedAmount<Rx>, TaggedAmount<Ry>);

    fn shares_amount(self, burned_lq: TaggedAmount<Lq>) -> (TaggedAmount<Rx>, TaggedAmount<Ry>);
}

impl AMMOps for ConstFnPool {
    fn output_amount(
        &self,
        base_asset: TaggedAssetClass<Base>,
        base_amount: TaggedAmount<Base>,
    ) -> TaggedAmount<Quote> {
        classic_cfmm_output_amount(
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
        classic_cfmm_reward_lp(
            self.reserves_x,
            self.reserves_y,
            self.liquidity,
            in_x_amount,
            in_y_amount,
        )
    }

    fn shares_amount(self, burned_lq: TaggedAmount<Lq>) -> (TaggedAmount<Rx>, TaggedAmount<Ry>) {
        classic_cfmm_shares_amount(self.reserves_x, self.reserves_y, self.liquidity, burned_lq)
    }
}

impl<Ctx> RequiresValidator<Ctx> for ConstFnPool
where
    Ctx: Has<DeployedValidator<{ ConstFnPoolV1 as u8 }>> + Has<DeployedValidator<{ ConstFnPoolV2 as u8 }>>,
{
    fn get_validator(&self, ctx: &Ctx) -> DeployedValidatorErased {
        match self.ver {
            ConstFnPoolVer::V1 => ctx
                .select::<DeployedValidator<{ ConstFnPoolV1 as u8 }>>()
                .erased(),
            _ => ctx
                .select::<DeployedValidator<{ ConstFnPoolV2 as u8 }>>()
                .erased(),
        }
    }
}

impl Pool for ConstFnPool {
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

impl<Ctx> TryFromLedger<BabbageTransactionOutput, Ctx> for ConstFnPool
where
    Ctx: Has<DeployedScriptHash<{ ConstFnPoolV1 as u8 }>>
        + Has<DeployedScriptHash<{ ConstFnPoolV2 as u8 }>>
        + Has<DeployedScriptHash<{ ConstFnPoolFeeSwitch as u8 }>>
        + Has<DeployedScriptHash<{ ConstFnPoolFeeSwitchBiDirFee as u8 }>>,
{
    fn try_from_ledger(repr: &BabbageTransactionOutput, ctx: &Ctx) -> Option<Self> {
        if let Some(pool_ver) = ConstFnPoolVer::try_from_address(repr.address(), ctx) {
            let value = repr.value();
            let pd = repr.datum().clone()?.into_pd()?;
            return match pool_ver {
                ConstFnPoolVer::V1 | ConstFnPoolVer::V2 => {
                    let conf = LegacyCFMMPoolConfig::try_from_pd(pd.clone())?;
                    let liquidity_neg = value.amount_of(conf.asset_lq.into())?;
                    Some(ConstFnPool {
                        id: PoolId::try_from(conf.pool_nft).ok()?,
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
                ConstFnPoolVer::FeeSwitch => {
                    let conf = FeeSwitchPoolConfig::try_from_pd(pd.clone())?;
                    let liquidity_neg = value.amount_of(conf.asset_lq.into())?;
                    Some(ConstFnPool {
                        id: PoolId::try_from(conf.pool_nft).ok()?,
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
                ConstFnPoolVer::FeeSwitchBiDirFee => {
                    let conf = FeeSwitchBidirectionalPoolConfig::try_from_pd(pd.clone())?;
                    let liquidity_neg = value.amount_of(conf.asset_lq.into())?;
                    Some(ConstFnPool {
                        id: PoolId::try_from(conf.pool_nft).ok()?,
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
        };
        None
    }
}

impl IntoLedger<TransactionOutput, ImmutablePoolUtxo> for ConstFnPool {
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

fn unsafe_update_datum(pool: &ConstFnPool, prev_datum: Option<DatumOption>) -> Option<DatumOption> {
    match prev_datum {
        Some(DatumOption::Datum {
            datum,
            len_encoding,
            tag_encoding,
            datum_tag_encoding,
            datum_bytes_encoding,
        }) => match pool.ver {
            ConstFnPoolVer::V1 | ConstFnPoolVer::V2 => Some(DatumOption::Datum {
                datum,
                len_encoding,
                tag_encoding,
                datum_tag_encoding,
                datum_bytes_encoding,
            }),
            ConstFnPoolVer::FeeSwitch | ConstFnPoolVer::FeeSwitchBiDirFee => {
                let mut cpd = datum.into_constr_pd()?;

                cpd.update_field_unsafe(6, pool.treasury_x.untag().into_pd());
                cpd.update_field_unsafe(7, pool.treasury_y.untag().into_pd());

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

impl ApplyOrder<ClassicalOnChainLimitSwap> for ConstFnPool {
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

impl ApplyOrder<ClassicalOnChainDeposit> for ConstFnPool {
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

impl ApplyOrder<ClassicalOnChainRedeem> for ConstFnPool {
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
