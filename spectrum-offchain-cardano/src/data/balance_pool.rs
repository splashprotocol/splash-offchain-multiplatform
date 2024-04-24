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
use cml_chain::utils::BigInteger;
use cml_chain::Value;
use cml_core::serialization::LenEncoding::{Canonical, Indefinite};
use cml_multi_era::babbage::BabbageTransactionOutput;
use num_integer::Roots;
use num_rational::Ratio;

use bloom_offchain::execution_engine::liquidity_book::pool::{Pool, PoolQuality};
use bloom_offchain::execution_engine::liquidity_book::side::{Side, SideM};
use bloom_offchain::execution_engine::liquidity_book::types::AbsolutePrice;
use spectrum_cardano_lib::ex_units::ExUnits;
use spectrum_cardano_lib::plutus_data::{ConstrPlutusDataExtension, DatumExtension};
use spectrum_cardano_lib::plutus_data::{IntoPlutusData, PlutusDataExtension};
use spectrum_cardano_lib::transaction::TransactionOutputExtension;
use spectrum_cardano_lib::types::TryFromPData;
use spectrum_cardano_lib::value::ValueExtension;
use spectrum_cardano_lib::{TaggedAmount, TaggedAssetClass};
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
use crate::deployment::{DeployedScriptInfo, DeployedValidator, DeployedValidatorErased, RequiresValidator};
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
    pub invariant: u128,
    pub invariant_length: u64,
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
            invariant: cpd.take_field(12)?.into_u128()?,
            invariant_length: cpd.take_field(13)?.into_u64()?,
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
        Ctx: Has<DeployedScriptInfo<{ BalanceFnPoolV1 as u8 }>>,
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
    pub invariant: u128,
    pub invariant_length: u64,
    pub ver: BalancePoolVer,
    /// How many execution units pool invokation costs.
    pub marginal_cost: ExUnits,
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
    ) -> (BigInteger, BigInteger, [BigInteger; 6]) {
        let (asset_reserves, asset_weight, lp_fee) = if base_asset_ac.untag() == self.asset_x.untag() {
            (
                (self.reserves_x.untag() - self.treasury_x.untag()) as f64,
                self.weight_x as f64,
                *self.lp_fee_x.numer() as f64,
            )
        } else {
            (
                (self.reserves_y.untag() - self.treasury_y.untag()) as f64,
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

        let new_balance_len = BigInteger::from(
            BigNumber::from(asset_reserves)
                .add(BigNumber::from(base_asset_in.untag() as f64))
                .to_string()
                .len(),
        );
        let g_len = BigInteger::from(BigNumber::from_str(&new_g.to_string()).unwrap().to_string().len());

        let new_t_pow_denum_len = BigInteger::from(
            BigNumber::from_str(&new_t.to_string())
                .unwrap()
                .pow(&BigNumber::from(WEIGHT_FEE_DEN))
                .to_string()
                .len(),
        );
        let new_t_pow_weight_len = BigInteger::from(
            BigNumber::from_str(&new_t.to_string())
                .unwrap()
                .pow(&BigNumber::from(asset_weight))
                .to_string()
                .len(),
        );

        // len( g ^ (weightDen) * feeDen)
        let g_equation_left_side_len = BigInteger::from(
            BigNumber::from_str(&new_g.to_string())
                .unwrap()
                .pow(&BigNumber::from(WEIGHT_FEE_DEN))
                .mul(BigNumber::from(FEE_DEN as f64))
                .to_string()
                .len(),
        );

        // (prevTokenBalance * feeDen + tokenDelta * fees)
        let right_side_len = BigInteger::from(
            BigNumber::from_str(&asset_reserves.to_string())
                .unwrap()
                .mul(BigNumber::from(FEE_DEN as f64))
                .add(
                    BigNumber::from(base_asset_in.untag() as f64)
                        .mul(BigNumber::from(lp_fee - *self.treasury_fee.numer() as f64)),
                )
                .to_string()
                .len(),
        );

        (
            new_g,
            new_t,
            [
                new_balance_len,
                g_len,
                new_t_pow_denum_len,
                new_t_pow_weight_len,
                g_equation_left_side_len,
                right_side_len,
            ],
        )
    }

    fn calculate_out_gt_values_for_swap(
        self,
        quote_asset_ac: TaggedAssetClass<Ry>,
        quote_asset_out: TaggedAmount<Ry>,
        precision: usize,
    ) -> (BigInteger, BigInteger, [BigInteger; 6]) {
        let (asset_reserves, asset_weight) = if quote_asset_ac.untag() == self.asset_x.untag() {
            (
                (self.reserves_x.untag() - self.treasury_x.untag()) as f64,
                self.weight_x as f64,
            )
        } else {
            (
                (self.reserves_y.untag() - self.treasury_y.untag()) as f64,
                self.weight_y as f64,
            )
        };
        let new_token_value =
            BigNumber::from(asset_reserves).sub(BigNumber::from(quote_asset_out.untag() as f64));
        // g = newTokenValue ^ (tokenWeight / commonWeightDenum)
        let new_g_raw =
            new_token_value.pow(&BigNumber::from(asset_weight).div(BigNumber::from(WEIGHT_FEE_DEN)));
        // t = newTokenValue ^ (1 / commonWeightDenum)
        let new_t_raw = new_token_value.pow(&BigNumber::from(1).div(BigNumber::from(WEIGHT_FEE_DEN)));
        // we should round raw value to max precision + additional round

        let new_g = round_big_number(new_g_raw, precision + ADDITIONAL_ROUND_PRECISION);
        let new_t = round_big_number(new_t_raw, precision + ADDITIONAL_ROUND_PRECISION);

        let new_balance_len = BigInteger::from(new_token_value.to_string().len());
        let g_len = BigInteger::from(BigNumber::from_str(&new_g.to_string()).unwrap().to_string().len());

        let new_t_pow_denum_len = BigInteger::from(
            BigNumber::from_str(&new_t.to_string())
                .unwrap()
                .pow(&BigNumber::from(WEIGHT_FEE_DEN))
                .to_string()
                .len(),
        );
        let new_t_pow_weight_len = BigInteger::from(
            BigNumber::from_str(&new_t.to_string())
                .unwrap()
                .pow(&BigNumber::from(asset_weight))
                .to_string()
                .len(),
        );

        let g_in_weight_degree_multiplicator = if (asset_weight == 1_f64) {
            BigNumber::from(1_f64)
        } else {
            BigNumber::from(WEIGHT_FEE_DEN) - BigNumber::from(asset_weight)
        };

        let g_in_weight_degree_mul_len = BigInteger::from(
            BigNumber::from_str(&new_g.to_string())
                .unwrap()
                .pow(&BigNumber::from(WEIGHT_FEE_DEN))
                .mul(&g_in_weight_degree_multiplicator)
                .to_string()
                .len(),
        );
        let new_assets_len = BigInteger::from(
            BigNumber::from(asset_reserves)
                .sub(BigNumber::from(quote_asset_out.untag() as f64))
                .to_string()
                .len(),
        );

        (
            new_g,
            new_t,
            [
                new_balance_len,
                g_len,
                new_t_pow_denum_len,
                new_t_pow_weight_len,
                g_in_weight_degree_mul_len,
                new_assets_len,
            ],
        )
    }

    // [BigInteger; 4]  - g and t related values
    // [BigInteger; 13] - lengths of variables in calculations
    fn construct_swap_related_values(
        self,
        base_asset_ac: TaggedAssetClass<Rx>,
        base_asset_in: TaggedAmount<Rx>,
        quote_asset_ac: TaggedAssetClass<Ry>,
        quote_asset_out: TaggedAmount<Ry>,
    ) -> ([BigInteger; 4], Vec<BigInteger>) {
        let x_length = (self.reserves_x.untag() - self.treasury_x.untag())
            .to_string()
            .len();
        let y_length = (self.reserves_y.untag() - self.treasury_y.untag())
            .to_string()
            .len();

        let max_precision = if x_length >= y_length { x_length } else { y_length };

        let (new_g_x, new_t_x, x_lengths) = if base_asset_ac.untag() == self.asset_x.untag() {
            self.calculate_in_gt_values_for_swap(base_asset_ac, base_asset_in, max_precision)
        } else {
            self.calculate_out_gt_values_for_swap(quote_asset_ac, quote_asset_out, max_precision)
        };

        let (new_g_y, new_t_y, y_lengths) = if base_asset_ac.untag() == self.asset_y.untag() {
            self.calculate_in_gt_values_for_swap(base_asset_ac, base_asset_in, max_precision)
        } else {
            self.calculate_out_gt_values_for_swap(quote_asset_ac, quote_asset_out, max_precision)
        };

        let gx_gy_length = BigInteger::from(
            BigNumber::from_str(&new_g_x.to_string())
                .unwrap()
                .mul(BigNumber::from_str(&new_g_y.to_string()).unwrap())
                .to_string()
                .len(),
        );

        let mut mut_arr = [x_lengths, y_lengths].concat();

        let mut common_vector = vec![gx_gy_length];

        common_vector.append(&mut mut_arr);

        ([new_g_x, new_t_x, new_g_y, new_t_y], common_vector)
    }

    // [gx, tx, gy, ty]
    fn create_redeemer(
        cfmmpool_action: CFMMPoolAction,
        pool_idx: u64,
        new_g_t: [BigInteger; 4],
        // contains lengths of variables in balance pool math calculations
        lengths: Vec<BigInteger>,
    ) -> PlutusData {
        /*
          Original structure of pool redeemer
            [ "action" ':= BalancePoolAction
            , "selfIx" ':= PInteger
            -- for swap, deposit / redeem (All assets) contains: gX, gY
            , "g"     ':= PBuiltinList (PAsData PInteger)
            -- for swap, deposit / redeem (All assets) contains: tX, tY
            , "t"     ':= PBuiltinList (PAsData PInteger)
            , "lengths" ':= PBuiltinList (PAsData PInteger)
            ]
        */

        let action_plutus_data = cfmmpool_action.to_plutus_data();
        let self_ix_pd = PlutusData::Integer(BigInteger::from(pool_idx));
        let g_list_pd = PlutusData::new_list(Vec::from([
            PlutusData::Integer(new_g_t[0].clone()),
            PlutusData::Integer(new_g_t[2].clone()),
        ]));
        let t_list_pd = PlutusData::new_list(Vec::from([
            PlutusData::Integer(new_g_t[1].clone()),
            PlutusData::Integer(new_g_t[3].clone()),
        ]));

        let processed_lengths_vector: Vec<PlutusData> = lengths
            .into_iter()
            .map(|value| PlutusData::Integer(value))
            .collect();

        let lengths_pd = PlutusData::new_list(processed_lengths_vector);

        PlutusData::ConstrPlutusData(ConstrPlutusData {
            alternative: 0,
            fields: Vec::from([action_plutus_data, self_ix_pd, g_list_pd, t_list_pd, lengths_pd]),
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
    Ctx: Has<DeployedScriptInfo<{ BalanceFnPoolV1 as u8 }>>,
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
                treasury_x: TaggedAmount::new(conf.treasury_x),
                treasury_y: TaggedAmount::new(conf.treasury_y),
                invariant: conf.invariant,
                invariant_length: conf.invariant_length,
                ver: pool_ver,
                marginal_cost: ctx.get().marginal_cost,
            });
        }
        None
    }
}

impl IntoLedger<TransactionOutput, ImmutablePoolUtxo> for BalancePool {
    fn into_ledger(self, mut immut_pool: ImmutablePoolUtxo) -> TransactionOutput {
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

        if let Some(DatumOption::Datum { datum, .. }) = &mut immut_pool.datum_option {
            unsafe_update_datum(
                datum,
                self.treasury_x.untag(),
                self.treasury_y.untag(),
                self.invariant,
                self.invariant_length,
            );
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

pub fn unsafe_update_datum(
    data: &mut PlutusData,
    treasury_x: u64,
    treasury_y: u64,
    invariant: u128,
    invariant_length: u64,
) {
    let cpd = data.get_constr_pd_mut().unwrap();
    cpd.set_field(8, treasury_x.into_pd());
    cpd.set_field(9, treasury_y.into_pd());
    cpd.set_field(12, invariant.into_pd());
    cpd.set_field(13, invariant_length.into_pd());
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

        let (gt_list, lengths_list): ([BigInteger; 4], Vec<BigInteger>) = match self.action {
            CFMMPoolAction::Swap => {
                let (base_asset_ac, base_asset, quote_asset_ac, quote_asset) =
                    // x -> y swap
                    if self.new_pool_state.reserves_x > self.prev_pool_state.reserves_x {
                        (TaggedAssetClass::new(self.new_pool_state.asset_x.untag()), TaggedAmount::new(x_delta), TaggedAssetClass::new(self.new_pool_state.asset_y.untag()), TaggedAmount::new(y_delta))
                    } else {
                        (TaggedAssetClass::new(self.new_pool_state.asset_y.untag()), TaggedAmount::new(y_delta), TaggedAssetClass::new(self.new_pool_state.asset_x.untag()), TaggedAmount::new(x_delta))
                    };
                self.prev_pool_state.construct_swap_related_values(
                    base_asset_ac,
                    base_asset,
                    quote_asset_ac,
                    quote_asset,
                )
            }
            CFMMPoolAction::Deposit => (
                [
                    BigInteger::from(0),
                    BigInteger::from(0),
                    BigInteger::from(0),
                    BigInteger::from(0),
                ],
                vec![],
            ),
            CFMMPoolAction::Redeem => (
                [
                    BigInteger::from(0),
                    BigInteger::from(0),
                    BigInteger::from(0),
                    BigInteger::from(0),
                ],
                vec![],
            ),
            CFMMPoolAction::Destroy => {
                unimplemented!();
            }
        };

        BalancePool::create_redeemer(self.action, self.pool_input_index, gt_list, lengths_list)
    }
}

pub fn round_big_number(orig_value: BigNumber, precision: usize) -> BigInteger {
    let int_part = orig_value.to_string().split(".").nth(0).unwrap().len();
    if (precision == 0) {
        BigInteger::from_str(
            orig_value.to_string().replace(".", "")[..(int_part)]
                .to_string()
                .as_str(),
        )
        .unwrap()
    } else {
        BigInteger::from_str(
            orig_value.to_string().replace(".", "")[..(precision)]
                .to_string()
                .as_str(),
        )
        .unwrap()
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
            (self.reserves_x - self.treasury_x),
            self.weight_x,
            (self.reserves_y - self.treasury_y),
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
        // Balance pool reward lp calculation is the same as for cfmm pool,
        // but we should "recalculate" change_x, change_y based on unlocked_lq
        let (unlocked_lq, change_x, change_y) = classic_cfmm_reward_lp(
            self.reserves_x - self.treasury_x,
            self.reserves_y - self.treasury_y,
            self.liquidity,
            in_x_amount,
            in_y_amount,
        );

        let x_to_deposit = (unlocked_lq.untag() as u128
            * (self.reserves_x.untag() - self.treasury_x.untag()) as u128)
            / (self.liquidity.untag() as u128);
        let y_to_deposit_bn = (unlocked_lq.untag() as u128
            * (self.reserves_y.untag() - self.treasury_y.untag()) as u128)
            / (self.liquidity.untag() as u128);

        (
            unlocked_lq,
            TaggedAmount::new(in_x_amount - x_to_deposit as u64),
            TaggedAmount::new(in_y_amount - y_to_deposit_bn as u64),
        )
    }

    fn shares_amount(self, burned_lq: TaggedAmount<Lq>) -> (TaggedAmount<Rx>, TaggedAmount<Ry>) {
        // Balance pool shares amount calculation is the same as for cfmm pool
        classic_cfmm_shares_amount(
            self.reserves_x - self.treasury_x,
            self.reserves_y - self.treasury_y,
            self.liquidity,
            burned_lq,
        )
    }
}

impl Pool for BalancePool {
    type U = ExUnits;
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
                self.treasury_y = TaggedAmount::new(
                    self.treasury_y.untag() + (input * self.treasury_fee.numer() / self.treasury_fee.denom()),
                );
                (output, self)
            }
            Side::Ask(input) => {
                // User ask is the opposite; sell the base asset for the quote asset.
                *base_reserves += input;
                *quote_reserves -= output;
                self.treasury_x = TaggedAmount::new(
                    self.treasury_x.untag() + (input * self.treasury_fee.numer() / self.treasury_fee.denom()),
                );
                (output, self)
            }
        }
    }

    fn quality(&self) -> PoolQuality {
        let lq = (self.reserves_x.untag() / self.weight_x) as u128
            * (self.reserves_y.untag() / self.weight_y) as u128;
        PoolQuality::from(lq.sqrt())
    }

    fn marginal_cost_hint(&self) -> Self::U {
        self.marginal_cost
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

        let invariant_num = round_big_number(
            BigNumber::from(self.reserves_x.untag() as f64)
                .sub(BigNumber::from(self.treasury_x.untag() as f64))
                .mul(BigNumber::from(self.invariant as f64)),
            0,
        )
        .as_u128()
        .unwrap();
        let invariant_denum = round_big_number(
            BigNumber::from(self.reserves_x.untag() as f64)
                .sub(BigNumber::from(self.treasury_x.untag() as f64))
                .sub(BigNumber::from(net_x as f64))
                .add(BigNumber::from(change_x.untag() as f64)),
            0,
        )
        .as_u128()
        .unwrap();

        let additional = if (invariant_num % invariant_denum == 0) {
            0
        } else {
            1
        };

        let new_invariant = (invariant_num / invariant_denum + additional);
        let new_invariant_length = (format!("{}", new_invariant)).len();

        self.liquidity = self.liquidity + unlocked_lq;
        self.invariant = new_invariant as u128;
        self.invariant_length = new_invariant_length as u64;

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

        let additional = if ((((self.reserves_x.untag() - x_amount.untag() - self.treasury_x.untag())
            as u128)
            * self.invariant as u128)
            % ((self.reserves_x.untag() - self.treasury_x.untag()) as u128)
            == 0)
        {
            0
        } else {
            1
        };

        let new_invariant = ((((self.reserves_x.untag() - x_amount.untag() - self.treasury_x.untag())
            as u128)
            * self.invariant as u128)
            / ((self.reserves_x.untag() - self.treasury_x.untag()) as u128))
            + additional;

        let new_invariant_length = (format!("{}", new_invariant)).len();

        self.invariant = new_invariant;
        self.invariant_length = new_invariant_length as u64;
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
    use cml_core::serialization::Serialize;
    use cml_crypto::ScriptHash;
    use num_rational::Ratio;
    use spectrum_cardano_lib::ex_units::ExUnits;
    use spectrum_cardano_lib::{AssetClass, AssetName, TaggedAmount, TaggedAssetClass};

    use spectrum_cardano_lib::types::TryFromPData;

    use crate::data::balance_pool::{BalancePool, BalancePoolConfig, BalancePoolRedeemer, BalancePoolVer};
    use crate::data::pool::CFMMPoolAction;
    use crate::data::PoolId;

    const DATUM_SAMPLE: &str = "d8799fd8799f581c5df8fe3f9f0e10855f930e0ea6c227e3bba0aba54d39f9d55b95e21c436e6674ffd8799f4040ff01d8799f581c4b3459fd18a1dbabe207cd19c9951a9fac9f5c0f9c384e3d97efba26457465737443ff04d8799f581c0df79145b95580c14ef4baf8d022d7f0cbb08f3bed43bf97a2ddd8cb426c71ff1a000186820a00009fd8799fd87a9f581cb046b660db0eaf9be4f4300180ccf277e4209dada77c48fbd37ba81dffffff581c8d4be10d934b60a22f267699ea3f7ebdade1f8e535d1bd0ef7ce18b61a0501bced08ff";

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
            invariant_length: 9,
            ver: BalancePoolVer::V1,
            marginal_cost: ExUnits {
                mem: 120000000,
                steps: 100000000000,
            },
        };
        let result = pool.swap(Side::Ask(100000000));
    }

    #[test]
    fn swap_redeemer_test() {
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
            reserves_x: TaggedAmount::new(200000000),
            weight_x: 1,
            reserves_y: TaggedAmount::new(84093845),
            weight_y: 4,
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
            treasury_x: TaggedAmount::new(10000),
            treasury_y: TaggedAmount::new(0),
            invariant: 99999999,
            invariant_length: 8,
            ver: BalancePoolVer::V1,
            marginal_cost: ExUnits {
                mem: 120000000,
                steps: 100000000000,
            },
        };

        let (result, new_pool) = pool.clone().swap(Side::Ask(100000000));

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
            weight_x: 1,
            reserves_y: TaggedAmount::new(100000000),
            weight_y: 4,
            liquidity: TaggedAmount::new(9223372036754775808),
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
            invariant: 99999999,
            invariant_length: 8,
            ver: BalancePoolVer::V1,
            marginal_cost: ExUnits {
                mem: 120000000,
                steps: 100000000000,
            },
        };

        let mut new_pool = pool.clone();

        new_pool.reserves_x = TaggedAmount::new(108000000);
        new_pool.reserves_y = TaggedAmount::new(108000000);
        new_pool.liquidity = TaggedAmount::new(9223372036746775809);

        let test_deposit_redeemer = BalancePoolRedeemer {
            pool_input_index: 0,
            action: CFMMPoolAction::Deposit,
            new_pool_state: new_pool,
            prev_pool_state: pool,
        }
        .to_plutus_data();

        assert_eq!(
            hex::encode(test_deposit_redeemer.to_canonical_cbor_bytes()),
            "d87985000082000082000080"
        )
    }
}
