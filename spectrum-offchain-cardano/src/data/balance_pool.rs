use std::fmt::Debug;
use std::ops::{Add, Div, Mul, Sub};
use std::str::FromStr;

use bignumber::BigNumber;
use cml_chain::address::Address;
use cml_chain::assets::MultiAsset;
use cml_chain::certs::StakeCredential;
use cml_chain::plutus::utils::ConstrPlutusDataEncoding;
use cml_chain::plutus::{ConstrPlutusData, PlutusData};
use cml_chain::transaction::{ConwayFormatTxOut, DatumOption, TransactionOutput};
use cml_chain::utils::BigInt;
use cml_chain::Value;
use cml_core::serialization::LenEncoding::{Canonical, Indefinite};
use cml_multi_era::babbage::BabbageTransactionOutput;
use log::info;
use num_integer::Roots;
use num_rational::Ratio;

use bloom_offchain::execution_engine::liquidity_book::pool::{Pool, PoolQuality};
use bloom_offchain::execution_engine::liquidity_book::side::{Side, SideM};
use bloom_offchain::execution_engine::liquidity_book::types::{AbsolutePrice, FeeAsset, InputAsset};
use spectrum_cardano_lib::plutus_data::{ConstrPlutusDataExtension, DatumExtension};
use spectrum_cardano_lib::plutus_data::{IntoPlutusData, PlutusDataExtension};
use spectrum_cardano_lib::transaction::TransactionOutputExtension;
use spectrum_cardano_lib::types::TryFromPData;
use spectrum_cardano_lib::value::ValueExtension;
use spectrum_cardano_lib::{AssetClass, TaggedAmount, TaggedAssetClass};
use spectrum_offchain::data::{Has, Stable};
use spectrum_offchain::ledger::{IntoLedger, TryFromLedger};

use crate::constants::{ADDITIONAL_ROUND_PRECISION, FEE_DEN, MAX_LQ_CAP, WEIGHT_FEE_DEN};
use crate::data::cfmm_pool::AMMOps;
use crate::data::deposit::ClassicalOnChainDeposit;
use crate::data::operation_output::{DepositOutput, RedeemOutput};
use crate::data::order::{Base, ClassicalOrder, PoolNft, Quote};
use crate::data::pair::order_canonical;
use crate::data::pool::{
    ApplyOrder, ApplyOrderError, AssetDeltas, CFMMPoolAction, ImmutablePoolUtxo, Lq, Rx, Ry,
};
use crate::data::redeem::ClassicalOnChainRedeem;
use crate::data::PoolId;
use crate::deployment::ProtocolValidator::BalanceFnPoolV1;
use crate::deployment::{DeployedScriptHash, DeployedValidator, DeployedValidatorErased, RequiresValidator};
use crate::pool_math::balance_math::balance_cfmm_output_amount;
use crate::pool_math::cfmm_math::{classic_cfmm_reward_lp, classic_cfmm_shares_amount};

#[derive(Debug)]
pub struct BalancePoolConfig {
    pub pool_nft: TaggedAssetClass<PoolNft>,
    pub asset_x: TaggedAssetClass<Rx>,
    pub asset_x_weight: u64,
    pub asset_y: TaggedAssetClass<Ry>,
    pub asset_y_weight: u64,
    pub asset_lq: TaggedAssetClass<Lq>,
    pub lp_fee_num: u64,
    pub treasury_fee_num: u64,
    pub treasury_x: u64,
    pub treasury_y: u64,
    pub invariant: u64,
}

impl TryFromPData for BalancePoolConfig {
    fn try_from_pd(data: PlutusData) -> Option<Self> {
        let mut cpd = data.into_constr_pd()?;
        Some(Self {
            pool_nft: TaggedAssetClass::try_from_pd(cpd.take_field(0)?)?,
            asset_x: TaggedAssetClass::try_from_pd(cpd.take_field(1)?)?,
            asset_x_weight: cpd.take_field(2)?.into_u64()?,
            asset_y: TaggedAssetClass::try_from_pd(cpd.take_field(3)?)?,
            asset_y_weight: cpd.take_field(4)?.into_u64()?,
            asset_lq: TaggedAssetClass::try_from_pd(cpd.take_field(5)?)?,
            lp_fee_num: cpd.take_field(6)?.into_u64()?,
            treasury_fee_num: cpd.take_field(7)?.into_u64()?,
            treasury_x: cpd.take_field(8)?.into_u64()?,
            treasury_y: cpd.take_field(9)?.into_u64()?,
            invariant: cpd.take_field(12)?.into_u64()?,
        })
    }
}

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub enum BalancePoolVer {
    V1,
}

impl BalancePoolVer {
    pub fn try_from_address<Ctx>(pool_addr: &Address, ctx: &Ctx) -> Option<BalancePoolVer>
    where
        Ctx: Has<DeployedScriptHash<{ BalanceFnPoolV1 as u8 }>>,
    {
        let maybe_hash = pool_addr.payment_cred().and_then(|c| match c {
            StakeCredential::PubKey { .. } => None,
            StakeCredential::Script { hash, .. } => Some(hash),
        });
        if let Some(this_hash) = maybe_hash {
            if ctx
                .select::<DeployedScriptHash<{ BalanceFnPoolV1 as u8 }>>()
                .unwrap()
                == *this_hash
            {
                return Some(BalancePoolVer::V1);
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
    pub invariant: u64,
    pub ver: BalancePoolVer,
}

impl BalancePool {
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

    fn calculate_in_gt_values_for_swap(
        self,
        base_asset_ac: TaggedAssetClass<Rx>,
        base_asset_in: TaggedAmount<Rx>,
        precision: usize,
    ) -> (BigInt, BigInt) {
        let (asset_reserves, asset_weight, lp_fee) = if base_asset_ac.untag() == self.asset_x.untag() {
            (
                self.reserves_x.untag() as f64,
                self.weight_x as f64,
                *self.lp_fee_x.numer() as f64,
            )
        } else {
            (
                self.reserves_y.untag() as f64,
                self.weight_y as f64,
                *self.lp_fee_y.numer() as f64,
            )
        };

        let new_token_value = BigNumber::from(asset_reserves).add(
            BigNumber::from(base_asset_in.untag() as f64)
                .mul(BigNumber::from(lp_fee - *self.treasury_fee.numer() as f64))
                .div(BigNumber::from(FEE_DEN as f64)),
        );
        // g = newTokenValue ^ (tokenWeight / commonWeightDenum)
        let new_g_raw =
            new_token_value.pow(&BigNumber::from(asset_weight).div(BigNumber::from(WEIGHT_FEE_DEN)));
        // t = newTokenValue ^ (1 / commonWeightDenum)
        let new_t_raw = new_token_value.pow(&BigNumber::from(1).div(BigNumber::from(WEIGHT_FEE_DEN)));

        // we should round raw value to max precision + additional round

        let new_g = round_big_number(new_g_raw, precision + ADDITIONAL_ROUND_PRECISION);
        let new_t = round_big_number(new_t_raw, precision + ADDITIONAL_ROUND_PRECISION);
        (new_g, new_t)
    }

    fn calculate_out_gt_values_for_swap(
        self,
        quote_asset_ac: TaggedAssetClass<Ry>,
        quote_asset_out: TaggedAmount<Ry>,
        precision: usize,
    ) -> (BigInt, BigInt) {
        let (asset_reserves, asset_weight) = if quote_asset_ac.untag() == self.asset_x.untag() {
            (self.reserves_x.untag() as f64, self.weight_x as f64)
        } else {
            (self.reserves_y.untag() as f64, self.weight_y as f64)
        };
        info!("asset_reserves {}", asset_reserves);
        info!("quote_asset_out {}", quote_asset_out.untag());
        let new_token_value = BigNumber::from(asset_reserves);
        // g = newTokenValue ^ (tokenWeight / commonWeightDenum)
        info!("new_token_value {}", new_token_value);
        let new_g_raw =
            new_token_value.pow(&BigNumber::from(asset_weight).div(BigNumber::from(WEIGHT_FEE_DEN)));
        // t = newTokenValue ^ (1 / commonWeightDenum)
        let new_t_raw = new_token_value.pow(&BigNumber::from(1).div(BigNumber::from(WEIGHT_FEE_DEN)));

        // we should round raw value to max precision + additional round

        let new_g = round_big_number(new_g_raw, precision + ADDITIONAL_ROUND_PRECISION);
        let new_t = round_big_number(new_t_raw, precision + ADDITIONAL_ROUND_PRECISION);
        (new_g, new_t)
    }

    fn calculate_gt_values_for_deposit(
        self,
        token_in_asset_ac: AssetClass,
        token_in: u64,
        precision: usize,
    ) -> (BigInt, BigInt) {
        let (asset_reserves, asset_weight) = if token_in_asset_ac == self.asset_x.untag() {
            (self.reserves_x.untag() as f64, self.weight_x as f64)
        } else {
            (self.reserves_y.untag() as f64, self.weight_y as f64)
        };

        let new_token_value = BigNumber::from(asset_reserves)
            .add(BigNumber::from(asset_reserves).add(BigNumber::from(token_in as f64)));
        // g = newTokenValue ^ (tokenWeight / commonWeightDenum)
        let new_g_raw =
            new_token_value.pow(&BigNumber::from(asset_weight).div(BigNumber::from(WEIGHT_FEE_DEN)));
        // t = newTokenValue ^ (1 / commonWeightDenum)
        let new_t_raw = new_token_value.pow(&BigNumber::from(1).div(BigNumber::from(WEIGHT_FEE_DEN)));

        // we should round raw value to max precision + additional round

        let new_g = round_big_number(new_g_raw, precision + ADDITIONAL_ROUND_PRECISION);
        let new_t = round_big_number(new_t_raw, precision + ADDITIONAL_ROUND_PRECISION);
        (new_g, new_t)
    }

    fn calculate_gt_values_for_redeem(
        self,
        token_in_asset_ac: AssetClass,
        token_in: u64,
        precision: usize,
    ) -> (BigInt, BigInt) {
        let (asset_reserves, asset_weight) = if token_in_asset_ac == self.asset_x.untag() {
            (self.reserves_x.untag() as f64, self.weight_x as f64)
        } else {
            (self.reserves_y.untag() as f64, self.weight_y as f64)
        };

        let new_token_value = BigNumber::from(asset_reserves)
            .add(BigNumber::from(asset_reserves).sub(BigNumber::from(token_in as f64)));
        // g = newTokenValue ^ (tokenWeight / commonWeightDenum)
        let new_g_raw =
            new_token_value.pow(&BigNumber::from(asset_weight).div(BigNumber::from(WEIGHT_FEE_DEN)));
        // t = newTokenValue ^ (1 / commonWeightDenum)
        let new_t_raw = new_token_value.pow(&BigNumber::from(1).div(BigNumber::from(WEIGHT_FEE_DEN)));

        // we should round raw value to max precision + additional round

        let new_g = round_big_number(new_g_raw, precision + ADDITIONAL_ROUND_PRECISION);
        let new_t = round_big_number(new_t_raw, precision + ADDITIONAL_ROUND_PRECISION);
        (new_g, new_t)
    }

    fn construct_g_and_t_for_swap(
        self,
        base_asset_ac: TaggedAssetClass<Rx>,
        base_asset_in: TaggedAmount<Rx>,
        quote_asset_ac: TaggedAssetClass<Ry>,
        quote_asset_out: TaggedAmount<Ry>,
    ) -> [BigInt; 4] {
        let x_length = self.reserves_x.untag().to_string().len();
        let y_length = self.reserves_y.untag().to_string().len();

        let max_precision = if x_length >= y_length { x_length } else { y_length };

        let (new_g_x, new_t_x) = if base_asset_ac.untag() == self.asset_x.untag() {
            self.calculate_in_gt_values_for_swap(base_asset_ac, base_asset_in, max_precision)
        } else {
            self.calculate_out_gt_values_for_swap(quote_asset_ac, quote_asset_out, max_precision)
        };

        let (new_g_y, new_t_y) = if base_asset_ac.untag() == self.asset_y.untag() {
            self.calculate_in_gt_values_for_swap(base_asset_ac, base_asset_in, max_precision)
        } else {
            self.calculate_out_gt_values_for_swap(quote_asset_ac, quote_asset_out, max_precision)
        };

        [new_g_x, new_t_x, new_g_y, new_t_y]
    }

    fn construct_g_and_t_for_deposit(
        self,
        token_x: TaggedAssetClass<Rx>,
        token_x_in: TaggedAmount<Rx>,
        token_y: TaggedAssetClass<Ry>,
        token_y_in: TaggedAmount<Ry>,
    ) -> [BigInt; 4] {
        let x_length = self.reserves_x.untag().to_string().len();
        let y_length = self.reserves_y.untag().to_string().len();

        let max_precision = if x_length >= y_length { x_length } else { y_length };

        let (new_g_x, new_t_x) =
            self.calculate_gt_values_for_deposit(token_x.untag(), token_x_in.untag(), max_precision);

        let (new_g_y, new_t_y) =
            self.calculate_gt_values_for_deposit(token_y.untag(), token_y_in.untag(), max_precision);

        [new_g_x, new_t_x, new_g_y, new_t_y]
    }

    fn construct_g_and_t_for_redeem(
        self,
        token_x: TaggedAssetClass<Rx>,
        token_x_in: TaggedAmount<Rx>,
        token_y: TaggedAssetClass<Ry>,
        token_y_in: TaggedAmount<Ry>,
    ) -> [BigInt; 4] {
        let x_length = self.reserves_x.untag().to_string().len();
        let y_length = self.reserves_y.untag().to_string().len();

        let max_precision = if x_length >= y_length { x_length } else { y_length };

        let (new_g_x, new_t_x) =
            self.calculate_gt_values_for_redeem(token_x.untag(), token_x_in.untag(), max_precision);

        let (new_g_y, new_t_y) =
            self.calculate_gt_values_for_redeem(token_y.untag(), token_y_in.untag(), max_precision);

        [new_g_x, new_t_x, new_g_y, new_t_y]
    }

    // [gx, tx, gy, ty]
    fn create_redeemer(
        cfmmpool_action: CFMMPoolAction,
        pool_idx: u64,
        new_g_t: [BigInt; 4],
    ) -> PlutusData {
        /*
          Original structure of pool redeemer
            [ "action" ':= BalancePoolAction
            , "selfIx" ':= PInteger
            -- for swap, deposit / redeem (All assets) contains: gX, gY
            , "g"     ':= PBuiltinList (PAsData PInteger)
            -- for swap, deposit / redeem (All assets) contains: tX, tY
            , "t"     ':= PBuiltinList (PAsData PInteger)
            ]
        */

        let action_plutus_data = cfmmpool_action.to_plutus_data();
        let self_ix_pd = PlutusData::Integer(BigInt::from(pool_idx));
        let g_list_pd = PlutusData::new_list(Vec::from([
            PlutusData::Integer(new_g_t[0].clone()),
            PlutusData::Integer(new_g_t[2].clone()),
        ]));
        let t_list_pd = PlutusData::new_list(Vec::from([
            PlutusData::Integer(new_g_t[1].clone()),
            PlutusData::Integer(new_g_t[3].clone()),
        ]));

        PlutusData::ConstrPlutusData(ConstrPlutusData {
            alternative: 0,
            fields: Vec::from([action_plutus_data, self_ix_pd, g_list_pd, t_list_pd]),
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

impl<Ctx> TryFromLedger<BabbageTransactionOutput, Ctx> for BalancePool
where
    Ctx: Has<DeployedScriptHash<{ BalanceFnPoolV1 as u8 }>>,
{
    fn try_from_ledger(repr: &BabbageTransactionOutput, ctx: &Ctx) -> Option<Self> {
        if let Some(pool_ver) = BalancePoolVer::try_from_address(repr.address(), ctx) {
            let value = repr.value();
            let pd = repr.datum().clone()?.into_pd()?;
            let conf = BalancePoolConfig::try_from_pd(pd.clone())?;
            let liquidity_neg = value.amount_of(conf.asset_lq.into())?;
            return Some(BalancePool {
                id: PoolId::try_from(conf.pool_nft).ok()?,
                reserves_x: TaggedAmount::new(value.amount_of(conf.asset_x.into())?),
                weight_x: conf.asset_x_weight,
                reserves_y: TaggedAmount::new(value.amount_of(conf.asset_y.into())?),
                weight_y: conf.asset_y_weight,
                liquidity: TaggedAmount::new(MAX_LQ_CAP - liquidity_neg),
                asset_x: conf.asset_x,
                asset_y: conf.asset_y,
                asset_lq: conf.asset_lq,
                lp_fee_x: Ratio::new_raw(conf.lp_fee_num, FEE_DEN),
                lp_fee_y: Ratio::new_raw(conf.lp_fee_num, FEE_DEN),
                treasury_fee: Ratio::new_raw(conf.treasury_fee_num, FEE_DEN),
                treasury_x: TaggedAmount::new(0),
                treasury_y: TaggedAmount::new(0),
                invariant: conf.invariant,
                ver: pool_ver,
            });
        }
        None
    }
}

impl IntoLedger<TransactionOutput, ImmutablePoolUtxo> for BalancePool {
    fn into_ledger(self, immut_pool: ImmutablePoolUtxo) -> TransactionOutput {
        println!("into_ledger");
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

        println!("going to update datum");

        TransactionOutput::new_conway_format_tx_out(ConwayFormatTxOut {
            address: immut_pool.address,
            amount: Value::new(coins, ma),
            datum_option: unsafe_update_datum(&self, immut_pool.datum_option),
            script_reference: immut_pool.script_reference,
            encodings: None,
        })
    }
}

pub fn unsafe_update_datum_pool(data: &mut PlutusData, treasury_x: u64, treasury_y: u64) {
    let cpd = data.get_constr_pd_mut().unwrap();
    cpd.set_field(8, treasury_x.into_pd());
    cpd.set_field(9, treasury_y.into_pd());
}

pub(crate) fn unsafe_update_datum(
    pool: &BalancePool,
    prev_datum: Option<DatumOption>,
) -> Option<DatumOption> {
    match prev_datum {
        Some(DatumOption::Datum {
            datum,
            len_encoding,
            tag_encoding,
            datum_tag_encoding,
            datum_bytes_encoding,
        }) => {
            let mut cpd = datum.into_constr_pd()?;

            cpd.update_field_unsafe(9, pool.treasury_x.untag().into_pd());
            cpd.update_field_unsafe(10, pool.treasury_y.untag().into_pd());

            info!("Updated datum tx {}", pool.treasury_x.untag());

            Some(DatumOption::Datum {
                datum: PlutusData::ConstrPlutusData(cpd),
                len_encoding,
                tag_encoding,
                datum_tag_encoding,
                datum_bytes_encoding,
            })
        }
        _ => panic!("Expected inline datum"),
    }
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
    Ctx: Has<DeployedValidator<{ BalanceFnPoolV1 as u8 }>>,
{
    fn get_validator(&self, ctx: &Ctx) -> DeployedValidatorErased {
        match self.ver {
            _ => ctx
                .select::<DeployedValidator<{ BalanceFnPoolV1 as u8 }>>()
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
        let x_delta = self
            .new_pool_state
            .reserves_x
            .untag()
            .abs_diff(self.prev_pool_state.reserves_x.untag());
        let y_delta = self
            .new_pool_state
            .reserves_y
            .untag()
            .abs_diff(self.prev_pool_state.reserves_y.untag());

        let gt_list: [BigInt; 4] = match self.action {
            CFMMPoolAction::Swap => {
                let (base_asset_ac, base_asset, quote_asset_ac, quote_asset) =
                    // x -> y swap
                    if self.new_pool_state.reserves_x > self.prev_pool_state.reserves_x {
                        (TaggedAssetClass::new(self.new_pool_state.asset_x.untag()), TaggedAmount::new(x_delta), TaggedAssetClass::new(self.new_pool_state.asset_y.untag()), TaggedAmount::new(y_delta))
                    } else {
                        (TaggedAssetClass::new(self.new_pool_state.asset_y.untag()), TaggedAmount::new(y_delta), TaggedAssetClass::new(self.new_pool_state.asset_x.untag()), TaggedAmount::new(x_delta))
                    };
                self.new_pool_state.construct_g_and_t_for_swap(
                    base_asset_ac,
                    base_asset,
                    quote_asset_ac,
                    quote_asset,
                )
            }
            CFMMPoolAction::Deposit => self.new_pool_state.construct_g_and_t_for_deposit(
                self.new_pool_state.asset_x,
                TaggedAmount::new(x_delta),
                self.new_pool_state.asset_y,
                TaggedAmount::new(y_delta),
            ),
            CFMMPoolAction::Redeem => self.new_pool_state.construct_g_and_t_for_redeem(
                self.new_pool_state.asset_x,
                TaggedAmount::new(x_delta),
                self.new_pool_state.asset_y,
                TaggedAmount::new(y_delta),
            ),
            CFMMPoolAction::Destroy => {
                unimplemented!();
            }
        };

        BalancePool::create_redeemer(self.action, self.pool_input_index, gt_list)
    }
}

pub fn round_big_number(orig_value: BigNumber, precision: usize) -> BigInt {
    info!("Orig value: {}", orig_value.to_string());
    let int_part = orig_value.to_string().split(".").nth(0).unwrap().len();
    info!("Orig value: {} int_part", int_part.to_string());
    info!(
        "replaced {}",
        orig_value.to_string().replace(".", "")[..(int_part + precision)].to_string()
    );
    BigInt::from_str(
        orig_value.to_string().replace(".", "")[..(int_part + precision)]
            .to_string()
            .as_str(),
    )
    .unwrap()
}

impl AMMOps for BalancePool {
    fn output_amount(
        &self,
        base_asset: TaggedAssetClass<Base>,
        base_amount: TaggedAmount<Base>,
    ) -> TaggedAmount<Quote> {
        println!("self.lp_fee_x: {}", self.lp_fee_x);
        println!("self.treasury_fee: {}", self.treasury_fee);
        println!("self.lp_fee_x - self.treasury_fee: {}", self.lp_fee_x - self.treasury_fee);
        balance_cfmm_output_amount(
            self.asset_x,
            self.reserves_x,
            self.weight_x,
            self.reserves_y,
            self.weight_y,
            base_asset,
            base_amount,
            self.lp_fee_x - self.treasury_fee,
            self.lp_fee_y - self.treasury_fee,
            self.invariant,
        )
    }

    fn reward_lp(
        &self,
        in_x_amount: u64,
        in_y_amount: u64,
    ) -> (TaggedAmount<Lq>, TaggedAmount<Rx>, TaggedAmount<Ry>) {
        // Balance pool reward lp calculation is the same as for cfmm pool
        classic_cfmm_reward_lp(
            self.reserves_x,
            self.reserves_y,
            self.liquidity,
            in_x_amount,
            in_y_amount,
        )
    }

    fn shares_amount(self, burned_lq: TaggedAmount<Lq>) -> (TaggedAmount<Rx>, TaggedAmount<Ry>) {
        // Balance pool shares amount calculation is the same as for cfmm pool
        classic_cfmm_shares_amount(self.reserves_x, self.reserves_y, self.liquidity, burned_lq)
    }
}

impl Pool for BalancePool {
    fn static_price(&self) -> AbsolutePrice {
        let x = self.asset_x.untag();
        let y = self.asset_y.untag();
        let [base, _] = order_canonical(x, y);
        if x == base {
            AbsolutePrice::new(
                self.reserves_y.untag() / self.weight_y,
                self.reserves_x.untag() / self.weight_x,
            )
        } else {
            AbsolutePrice::new(
                self.reserves_x.untag() / self.weight_x,
                self.reserves_y.untag() / self.weight_y,
            )
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
        println!("Pool: {:?}", self);
        println!("Input: {:?}", input);
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
        println!("Base: {}, Quote: {}", base, quote);
        println!("Calculated output: {}", output);
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
                self.treasury_y = TaggedAmount::new(self.treasury_y.untag() + (input * self.treasury_fee.numer() / self.treasury_fee.denom()));
                (output, self)
            }
            Side::Ask(input) => {
                // User ask is the opposite; sell the base asset for the quote asset.
                *base_reserves += input;
                *quote_reserves -= output;
                self.treasury_x = TaggedAmount::new(self.treasury_x.untag() + (input * self.treasury_fee.numer() / self.treasury_fee.denom()));
                println!("self.treasury_x {}", self.treasury_x.untag());
                (output, self)
            }
        }
    }

    fn quality(&self) -> PoolQuality {
        let lq =
            ((self.reserves_x.untag() / self.weight_x) * (self.reserves_y.untag() / self.weight_y)).sqrt();
        PoolQuality(self.static_price(), lq)
    }
}

impl ApplyOrder<ClassicalOnChainDeposit> for BalancePool {
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

impl ApplyOrder<ClassicalOnChainRedeem> for BalancePool {
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

mod tests {
    use bloom_offchain::execution_engine::liquidity_book::pool::Pool;
    use bloom_offchain::execution_engine::liquidity_book::side::Side;
    use cml_chain::plutus::PlutusData;
    use cml_chain::Deserialize;
    use cml_crypto::ScriptHash;
    use num_rational::Ratio;
    use spectrum_cardano_lib::{AssetClass, AssetName, TaggedAmount, TaggedAssetClass};

    use spectrum_cardano_lib::types::TryFromPData;

    use crate::data::balance_pool::{BalancePool, BalancePoolConfig, BalancePoolVer};
    use crate::data::PoolId;

    const DATUM_SAMPLE: &str = "d8799fd8799f581cdd061b480daddd9a833d2477c791356be4e134a433e19df7eb18be10504f534f43494554595f4144415f4e4654ffd8799f4040ff08d8799f581c279f842c33eed9054b9e3c70cd6a3b32298259c24b78b895cb41d91a4454554e41ff02d8799f581cc44de4596c7f4d600b631fab7ef1363331168463d4229cbc75ca18894c4b534f43494554595f5f4c51ff1a00018574181e000080581cdb73d7b28075e8869bb862857ded32d9b6fe9420d95aa94f5d34f80a01ff";

    #[test]
    fn parse_balance_pool_datum() {
        let pd = PlutusData::from_cbor_bytes(&*hex::decode(DATUM_SAMPLE).unwrap()).unwrap();
        let maybe_conf = BalancePoolConfig::try_from_pd(pd);
        assert!(maybe_conf.is_some())
    }

    #[test]
    fn swap() {
        let pool = BalancePool {
            id: PoolId::from((
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
            reserves_x: TaggedAmount::new(100000000),
            weight_x: 2,
            reserves_y: TaggedAmount::new(100000000),
            weight_y: 8,
            liquidity: TaggedAmount::new(0),
            asset_x: TaggedAssetClass::new(AssetClass::Native),
            asset_y: TaggedAssetClass::new(AssetClass::Token((
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
            asset_lq: TaggedAssetClass::new(AssetClass::Token((
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
            lp_fee_x: Ratio::new_raw(99970, 100000),
            lp_fee_y: Ratio::new_raw(99970, 100000),
            treasury_fee: Ratio::new_raw(10, 100000),
            treasury_x: TaggedAmount::new(0),
            treasury_y: TaggedAmount::new(0),
            invariant: 100000000,
            ver: BalancePoolVer::V1,
        };
        let result = pool.swap(Side::Ask(100000000));
    }
}
