use std::fmt::Debug;

use cml_chain::address::Address;
use cml_chain::assets::MultiAsset;
use cml_chain::certs::StakeCredential;
use cml_chain::plutus::{ConstrPlutusData, PlutusData};
use cml_chain::plutus::utils::ConstrPlutusDataEncoding;
use cml_chain::transaction::{ConwayFormatTxOut, DatumOption, TransactionOutput};
use cml_chain::utils::BigInteger;
use cml_chain::Value;
use cml_core::serialization::LenEncoding::{Canonical, Indefinite};
use cml_multi_era::babbage::BabbageTransactionOutput;
use num_rational::Ratio;
use num_traits::{Pow, ToPrimitive};
use primitive_types::U512;

use bloom_offchain::execution_engine::liquidity_book::pool::{Pool, PoolQuality};
use bloom_offchain::execution_engine::liquidity_book::side::{Side, SideM};
use bloom_offchain::execution_engine::liquidity_book::types::AbsolutePrice;
use spectrum_cardano_lib::{TaggedAmount, TaggedAssetClass};
use spectrum_cardano_lib::ex_units::ExUnits;
use spectrum_cardano_lib::plutus_data::{ConstrPlutusDataExtension, DatumExtension};
use spectrum_cardano_lib::plutus_data::{IntoPlutusData, PlutusDataExtension};
use spectrum_cardano_lib::transaction::TransactionOutputExtension;
use spectrum_cardano_lib::types::TryFromPData;
use spectrum_cardano_lib::value::ValueExtension;
use spectrum_offchain::data::{Has, Stable};
use spectrum_offchain::ledger::{IntoLedger, TryFromLedger};

use crate::constants::{FEE_DEN, MAX_LQ_CAP};
use crate::data::cfmm_pool::AMMOps;
use crate::data::deposit::ClassicalOnChainDeposit;
use crate::data::operation_output::{DepositOutput, RedeemOutput};
use crate::data::order::{Base, ClassicalOrder, PoolNft, Quote};
use crate::data::pair::order_canonical;
use crate::data::pool::{
    ApplyOrder, ApplyOrderError, AssetDeltas, CFMMPoolAction, ImmutablePoolUtxo, Lq, Rx, Ry,
};
use crate::data::PoolId;
use crate::data::redeem::ClassicalOnChainRedeem;
use crate::deployment::{DeployedScriptInfo, DeployedValidator, DeployedValidatorErased, RequiresValidator};
use crate::deployment::ProtocolValidator::StableFnPoolT2T;
use crate::pool_math::cfmm_math::{classic_cfmm_reward_lp, classic_cfmm_shares_amount};
use crate::pool_math::stable_pool_t2t_exact_math::{calc_stable_swap, calculate_invariant};

const N_TRADABLE_ASSETS: u64 = 2;

#[derive(Debug)]
pub struct StablePoolT2TConfig {
    pub pool_nft: TaggedAssetClass<PoolNft>,
    // max 16k
    pub an2n: u64,
    pub asset_x: TaggedAssetClass<Rx>,
    pub asset_y: TaggedAssetClass<Ry>,
    pub multiplier_x: u64,
    pub multiplier_y: u64,
    pub asset_lq: TaggedAssetClass<Lq>,
    pub lp_fee_num: u64,
    pub treasury_fee_num: u64,
    pub treasury_x: u64,
    pub treasury_y: u64,
}

struct DatumMapping {
    pub pool_nft: usize,
    pub an2n: usize,
    pub asset_x: usize,
    pub asset_y: usize,
    pub multiplier_x: usize,
    pub multiplier_y: usize,
    pub asset_lq: usize,
    pub lp_fee_num: usize,
    pub treasury_fee_num: usize,
    pub treasury_x: usize,
    pub treasury_y: usize,
}

const DATUM_MAPPING: DatumMapping = DatumMapping {
    pool_nft: 0,
    an2n: 1,
    asset_x: 2,
    asset_y: 3,
    multiplier_x: 4,
    multiplier_y: 5,
    asset_lq: 6,
    lp_fee_num: 9,
    treasury_fee_num: 10,
    treasury_x: 13,
    treasury_y: 14,
};

impl TryFromPData for StablePoolT2TConfig {
    fn try_from_pd(data: PlutusData) -> Option<Self> {
        let mut cpd = data.into_constr_pd()?;
        Some(Self {
            pool_nft: TaggedAssetClass::try_from_pd(cpd.take_field(DATUM_MAPPING.pool_nft)?)?,
            an2n: cpd.take_field(DATUM_MAPPING.an2n)?.into_u64()?,
            asset_x: TaggedAssetClass::try_from_pd(cpd.take_field(DATUM_MAPPING.asset_x)?)?,
            asset_y: TaggedAssetClass::try_from_pd(cpd.take_field(DATUM_MAPPING.asset_y)?)?,
            multiplier_x: cpd.take_field(DATUM_MAPPING.multiplier_x)?.into_u64()?,
            multiplier_y: cpd.take_field(DATUM_MAPPING.multiplier_y)?.into_u64()?,
            asset_lq: TaggedAssetClass::try_from_pd(cpd.take_field(DATUM_MAPPING.asset_lq)?)?,
            lp_fee_num: cpd.take_field(DATUM_MAPPING.lp_fee_num)?.into_u64()?,
            treasury_fee_num: cpd.take_field(DATUM_MAPPING.treasury_fee_num)?.into_u64()?,
            treasury_x: cpd.take_field(DATUM_MAPPING.treasury_x)?.into_u64()?,
            treasury_y: cpd.take_field(DATUM_MAPPING.treasury_y)?.into_u64()?,
        })
    }
}

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub enum StablePoolT2TVer {
    V1,
}

impl StablePoolT2TVer {
    pub fn try_from_address<Ctx>(pool_addr: &Address, ctx: &Ctx) -> Option<StablePoolT2TVer>
    where
        Ctx: Has<DeployedScriptInfo<{ StableFnPoolT2T as u8 }>>,
    {
        let maybe_hash = pool_addr.payment_cred().and_then(|c| match c {
            StakeCredential::PubKey { .. } => None,
            StakeCredential::Script { hash, .. } => Some(hash),
        });
        if let Some(this_hash) = maybe_hash {
            if ctx
                .select::<DeployedScriptInfo<{ StableFnPoolT2T as u8 }>>()
                .script_hash
                == *this_hash
            {
                return Some(StablePoolT2TVer::V1);
            }
        };
        None
    }
}

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub struct StablePoolT2T {
    pub id: PoolId,
    pub an2n: u64,
    pub reserves_x: TaggedAmount<Rx>,
    pub multiplier_x: u64,
    pub reserves_y: TaggedAmount<Ry>,
    pub multiplier_y: u64,
    pub liquidity: TaggedAmount<Lq>,
    pub asset_x: TaggedAssetClass<Rx>,
    pub asset_y: TaggedAssetClass<Ry>,
    pub asset_lq: TaggedAssetClass<Lq>,
    pub lp_fee_x: Ratio<u64>,
    pub lp_fee_y: Ratio<u64>,
    pub treasury_fee: Ratio<u64>,
    pub treasury_x: TaggedAmount<Rx>,
    pub treasury_y: TaggedAmount<Ry>,
    pub ver: StablePoolT2TVer,
    /// How many execution units pool invokation costs.
    pub marginal_cost: ExUnits,
}

impl StablePoolT2T {

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

    // [gx, tx, gy, ty]
    fn create_redeemer(pool_action: CFMMPoolAction, pool_in_idx: u64, pool_out_idx: u64) -> PlutusData {
        /*
          Original structure of pool redeemer

            pub type PoolAction {
              AMMAction { context_values_list: List<Int> }
              // "context_values_list" for swap actions contains:
              // - 0: Value of the StableSwap invariant for the output state;
              PDAOAction
            }

            pub type PoolRedeemer {
                pool_in_ix: Int,
                pool_out_ix: Int,
                action: PoolAction,
            }
        */

        let self_ix_pd = PlutusData::Integer(BigInteger::from(pool_in_idx));
        let self_out_pd = PlutusData::Integer(BigInteger::from(pool_out_idx));

        let context_values_list = PlutusData::ConstrPlutusData(ConstrPlutusData {
            alternative: 0,
            fields: Vec::from([]),
            encodings: Some(ConstrPlutusDataEncoding {
                len_encoding: Canonical,
                tag_encoding: Some(cbor_event::Sz::One),
                alternative_encoding: None,
                fields_encoding: Indefinite,
                prefer_compact: true,
            }),
        });

        PlutusData::ConstrPlutusData(ConstrPlutusData {
            alternative: 0,
            fields: Vec::from([self_ix_pd, self_out_pd, context_values_list]),
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

impl<Ctx> TryFromLedger<BabbageTransactionOutput, Ctx> for StablePoolT2T
where
    Ctx: Has<DeployedScriptInfo<{ StableFnPoolT2T as u8 }>>,
{
    fn try_from_ledger(repr: &BabbageTransactionOutput, ctx: &Ctx) -> Option<Self> {
        if let Some(pool_ver) = StablePoolT2TVer::try_from_address(repr.address(), ctx) {
            let value = repr.value();
            let pd = repr.datum().clone()?.into_pd()?;
            let conf = StablePoolT2TConfig::try_from_pd(pd.clone())?;
            let liquidity_neg = value.amount_of(conf.asset_lq.into())?;
            return Some(StablePoolT2T {
                id: PoolId::try_from(conf.pool_nft).ok()?,
                an2n: conf.an2n,
                reserves_x: TaggedAmount::new(value.amount_of(conf.asset_x.into())?),
                multiplier_x: conf.multiplier_x,
                reserves_y: TaggedAmount::new(value.amount_of(conf.asset_y.into())?),
                multiplier_y: conf.multiplier_y,
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
                marginal_cost: ctx.get().marginal_cost,
            });
        }
        None
    }
}

impl IntoLedger<TransactionOutput, ImmutablePoolUtxo> for StablePoolT2T {
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
    cpd.set_field(DATUM_MAPPING.treasury_x, treasury_x.into_pd());
    cpd.set_field(DATUM_MAPPING.treasury_y, treasury_y.into_pd());
}

impl Stable for StablePoolT2T {
    type StableId = PoolId;
    fn stable_id(&self) -> Self::StableId {
        self.id
    }
    fn is_quasi_permanent(&self) -> bool {
        true
    }
}

impl<Ctx> RequiresValidator<Ctx> for StablePoolT2T
where
    Ctx: Has<DeployedValidator<{ StableFnPoolT2T as u8 }>>,
{
    fn get_validator(&self, ctx: &Ctx) -> DeployedValidatorErased {
        match self.ver {
            _ => ctx
                .select::<DeployedValidator<{ StableFnPoolT2T as u8 }>>()
                .erased(),
        }
    }
}

pub struct StablePoolRedeemer {
    pub pool_input_index: u64,
    pub action: CFMMPoolAction,
    pub new_pool_state: StablePoolT2T,
    pub prev_pool_state: StablePoolT2T,
}

impl StablePoolRedeemer {
    pub fn to_plutus_data(self) -> PlutusData {
        StablePoolT2T::create_redeemer(self.action, self.pool_input_index)
    }
}

impl AMMOps for StablePoolT2T {
    fn output_amount(
        &self,
        base_asset: TaggedAssetClass<Base>,
        base_amount: TaggedAmount<Base>,
    ) -> TaggedAmount<Quote> {
        calc_stable_swap(
            self.asset_x,
            self.reserves_x - self.treasury_x,
            self.multiplier_x,
            self.reserves_y - self.treasury_y,
            self.multiplier_y,
            base_asset,
            base_amount,
            self.an2n,
        )
    }

    fn reward_lp(
        &self,
        in_x_amount: u64,
        in_y_amount: u64,
    ) -> (TaggedAmount<Lq>, TaggedAmount<Rx>, TaggedAmount<Ry>) {
        classic_cfmm_reward_lp(
            self.reserves_x - self.treasury_x,
            self.reserves_y - self.treasury_y,
            self.liquidity,
            in_x_amount,
            in_y_amount,
        )
    }

    fn shares_amount(self, burned_lq: TaggedAmount<Lq>) -> (TaggedAmount<Rx>, TaggedAmount<Ry>) {
        classic_cfmm_shares_amount(
            self.reserves_x - self.treasury_x,
            self.reserves_y - self.treasury_y,
            self.liquidity,
            burned_lq,
        )
    }
}

impl Pool for StablePoolT2T {
    type U = ExUnits;
    fn static_price(&self) -> AbsolutePrice {
        let x = self.asset_x.untag();
        let y = self.asset_y.untag();

        let x_calc = (self.reserves_x.untag() - self.treasury_x.untag()) * self.multiplier_x;
        let y_calc = (self.reserves_y.untag() - self.treasury_y.untag()) * self.multiplier_y;

        let nn = N_TRADABLE_ASSETS.pow(N_TRADABLE_ASSETS as u32);
        let ann = self.an2n / nn;
        let d = calculate_invariant(&U512::from(x_calc), &U512::from(y_calc), &U512::from(self.an2n));

        let dn1 = vec![d; usize::try_from(N_TRADABLE_ASSETS + 1).unwrap()]
            .iter()
            .copied()
            .reduce(|a, b| a * b)
            .unwrap();
        let price_num = ann + (dn1 / U512::from(nn * x_calc.pow(2) * y_calc)).as_u64();
        let price_denom = self.treasury_fee.denom().to_u64().unwrap()
            * (ann + (dn1 / U512::from(nn * y_calc.pow(2) * x_calc)).as_u64());

        let [base, _] = order_canonical(x, y);
        if x == base {
            let reversed_total_fee_num_x =
                self.treasury_fee.denom() - self.lp_fee_x.numer() - self.treasury_fee.numer();

            AbsolutePrice::new(price_num * reversed_total_fee_num_x, price_denom)
        } else {
            let reversed_total_fee_num_y =
                self.treasury_fee.denom() - self.lp_fee_y.numer() - self.treasury_fee.numer();
            AbsolutePrice::new(price_denom, reversed_total_fee_num_y * price_num)
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
        let pure_output = match input {
            Side::Bid(input) => self
                .output_amount(TaggedAssetClass::new(quote), TaggedAmount::new(input))
                .untag(),
            Side::Ask(input) => self
                .output_amount(TaggedAssetClass::new(base), TaggedAmount::new(input))
                .untag(),
        };
        let (base_reserves, lp_fee, quote_reserves) = if x == base {
            (self.reserves_x.as_mut(), self.lp_fee_y, self.reserves_y.as_mut())
        } else {
            (self.reserves_y.as_mut(), self.lp_fee_x, self.reserves_x.as_mut())
        };
        let lp_fees = pure_output * lp_fee.numer() / lp_fee.denom();
        let treasury_fee = pure_output * self.treasury_fee.numer() / self.treasury_fee.denom();
        let output = pure_output - treasury_fee - lp_fees;
        match input {
            Side::Bid(input) => {
                // A user bid means that they wish to buy the base asset for the quote asset, hence
                // pool reserves of base decreases while reserves of quote increase.
                *quote_reserves += input;
                *base_reserves -= output;
                self.treasury_y = TaggedAmount::new(self.treasury_y.untag() + treasury_fee);
                (output, self)
            }
            Side::Ask(input) => {
                // User ask is the opposite; sell the base asset for the quote asset.
                *base_reserves += input;
                *quote_reserves -= output;
                self.treasury_x = TaggedAmount::new(self.treasury_x.untag() + treasury_fee);
                (output, self)
            }
        }
    }

    fn quality(&self) -> PoolQuality {
        // todo: example below
        // let lq = (self.reserves_x.untag() / self.weight_x) as u128
        //     * (self.reserves_y.untag() / self.weight_y) as u128;
        // PoolQuality::from(lq.sqrt())
        unimplemented!()
    }

    fn marginal_cost_hint(&self) -> Self::U {
        self.marginal_cost
    }

    fn swaps_allowed(&self) -> bool {
        // balance pools do not support lq bound, so
        // swaps allowed all time
        true
    }
}

impl ApplyOrder<ClassicalOnChainDeposit> for StablePoolT2T {
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

impl ApplyOrder<ClassicalOnChainRedeem> for StablePoolT2T {
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

#[cfg(test)]
mod tests {
    use bloom_offchain::execution_engine::bundled::Bundled;
    use bloom_offchain::execution_engine::liquidity_book::pool::Pool;
    use bloom_offchain::execution_engine::liquidity_book::side::Side;
    use cml_chain::plutus::PlutusData;
    use cml_chain::Deserialize;
    use cml_core::serialization::Serialize;
    use cml_crypto::{Ed25519KeyHash, ScriptHash, TransactionHash};
    use num_rational::Ratio;
    use primitive_types::U512;
    use spectrum_cardano_lib::ex_units::ExUnits;
    use spectrum_cardano_lib::output::FinalizedTxOut;
    use spectrum_cardano_lib::{AssetClass, AssetName, OutputRef, TaggedAmount, TaggedAssetClass};
    use std::ops::Sub;

    use spectrum_cardano_lib::types::TryFromPData;

    use crate::data::balance_pool::{BalancePool, BalancePoolConfig, BalancePoolRedeemer, BalancePoolVer};
    use crate::data::order::ClassicalOrder;
    use crate::data::order::OrderType::BalanceFn;
    use crate::data::pool::{ApplyOrder, CFMMPoolAction, Lq, Rx, Ry};
    use crate::data::redeem::{ClassicalOnChainRedeem, Redeem};
    use crate::data::{OnChainOrderId, PoolId};
    use crate::data::stable_pool_t2t::StablePoolT2TConfig;

    const DATUM_SAMPLE: &str = "d8799fd8799f581c7dbe6f0c7849e2dae806cd4681910bfe1bbc0d5fd4e370e8e2f7bd4a436e6674ff190c80d8799f4040ffd8799f581c4b3459fd18a1dbabe207cd19c9951a9fac9f5c0f9c384e3d97efba26457465737443ff0000d8799f581c6abe65f6adc8301ff4dbfcfcec1a187075639d21f85cae3c1cf2a060426c71ffd87980d879801a000186820a581c4b3459fd18a1dbabe207cd19c9951a9fac9f5c0f9c384e3d97efba26581c4b3459fd18a1dbabe207cd19c9951a9fac9f5c0f9c384e3d97efba260000ff";

    #[test]
    fn parse_stable_pool_t2t_datum() {
        let pd = PlutusData::from_cbor_bytes(&*hex::decode(DATUM_SAMPLE).unwrap()).unwrap();
        let maybe_conf = StablePoolT2TConfig::try_from_pd(pd);
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
            reserves_x: TaggedAmount::new(2115301811439),
            weight_x: 1,
            reserves_y: TaggedAmount::new(27887555508598),
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
            lp_fee_x: Ratio::new_raw(99000, 100000),
            lp_fee_y: Ratio::new_raw(99000, 100000),
            treasury_fee: Ratio::new_raw(100, 100000),
            treasury_x: TaggedAmount::new(1143236614),
            treasury_y: TaggedAmount::new(3057757049),
            ver: BalancePoolVer::V1,
            marginal_cost: ExUnits {
                mem: 120000000,
                steps: 100000000000,
            },
        };
        let result = pool.swap(Side::Ask(200000000));

        assert_eq!(result.0, 652178037)
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
            ver: BalancePoolVer::V1,
            marginal_cost: ExUnits {
                mem: 120000000,
                steps: 100000000000,
            },
        };

        let (_result, new_pool) = pool.clone().swap(Side::Ask(363613802862));

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
        let pool_id = PoolId::from((
            ScriptHash::from([
                162, 206, 112, 95, 150, 240, 52, 167, 61, 102, 158, 92, 11, 47, 25, 41, 48, 224, 188, 211,
                138, 203, 127, 107, 246, 89, 115, 157,
            ]),
            AssetName::from((
                3,
                [
                    110, 102, 116, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
                    0, 0, 0, 0,
                ],
            )),
        ));

        let token_x = TaggedAssetClass::new(AssetClass::Native);
        let token_y = TaggedAssetClass::new(AssetClass::Token((
            ScriptHash::from([
                75, 52, 89, 253, 24, 161, 219, 171, 226, 7, 205, 25, 201, 149, 26, 159, 172, 159, 92, 15,
                156, 56, 78, 61, 151, 239, 186, 38,
            ]),
            AssetName::from((
                5,
                [
                    116, 101, 115, 116, 67, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
                    0, 0, 0, 0, 0,
                ],
            )),
        )));

        let token_lq = TaggedAssetClass::new(AssetClass::Token((
            ScriptHash::from([
                114, 191, 27, 172, 195, 20, 1, 41, 111, 158, 228, 210, 254, 123, 132, 165, 36, 56, 38, 251,
                3, 233, 206, 25, 51, 218, 254, 192,
            ]),
            AssetName::from((
                2,
                [
                    108, 113, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
                    0, 0, 0, 0,
                ],
            )),
        )));

        let pool = BalancePool {
            id: pool_id,
            reserves_x: TaggedAmount::new(29655901),
            weight_x: 1,
            reserves_y: TaggedAmount::new(53144),
            weight_y: 4,
            liquidity: TaggedAmount::new(9223372036854587823),
            asset_x: token_x,
            asset_y: token_y,
            asset_lq: token_lq,
            lp_fee_x: Ratio::new_raw(99000, 100000),
            lp_fee_y: Ratio::new_raw(99000, 100000),
            treasury_fee: Ratio::new_raw(100, 100000),
            treasury_x: TaggedAmount::new(13000),
            treasury_y: TaggedAmount::new(94),
            ver: BalancePoolVer::V1,
            marginal_cost: ExUnits {
                mem: 120000000,
                steps: 100000000000,
            },
        };

        const TX: &str = "6c038a69587061acd5611507e68b1fd3a7e7d189367b7853f3bb5079a118b880";
        const IX: u64 = 1;

        let test_order: ClassicalOnChainRedeem = ClassicalOrder {
            id: OnChainOrderId(OutputRef::new(TransactionHash::from_hex(TX).unwrap(), IX)),
            pool_id: pool_id,
            order: Redeem {
                pool_nft: pool_id,
                token_x: token_x,
                token_y: token_y,
                token_lq: token_lq,
                token_lq_amount: TaggedAmount::new(1900727),
                ex_fee: 1500000,
                reward_pkh: Ed25519KeyHash::from([0u8; 28]),
                reward_stake_pkh: None,
                collateral_ada: 3000000,
                order_type: BalanceFn,
            },
        };

        let test = pool.apply_order(test_order);

        let res = test.map(|res| println!("{:?}", res.0));

        assert_eq!(1, 1)
    }
}
