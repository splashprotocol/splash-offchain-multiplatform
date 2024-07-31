use std::fmt::Debug;

use cml_chain::address::Address;
use cml_chain::assets::MultiAsset;
use cml_chain::certs::StakeCredential;
use cml_chain::plutus::{ConstrPlutusData, PlutusData};
use cml_chain::transaction::{ConwayFormatTxOut, TransactionOutput};
use cml_chain::utils::BigInteger;
use cml_chain::Value;
use cml_multi_era::babbage::BabbageTransactionOutput;
use type_equalities::IsEqual;

use bloom_offchain::execution_engine::liquidity_book::core::{Next, Unit};
use bloom_offchain::execution_engine::liquidity_book::market_maker::{
    AbsoluteReserves, MakerBehavior, MarketMaker, PoolQuality, SpotPrice,
};
use bloom_offchain::execution_engine::liquidity_book::side::{OnSide, Side};
use bloom_offchain::execution_engine::liquidity_book::types::AbsolutePrice;
use spectrum_cardano_lib::{TaggedAmount, TaggedAssetClass};
use spectrum_cardano_lib::ex_units::ExUnits;
use spectrum_cardano_lib::plutus_data::{
    ConstrPlutusDataExtension, DatumExtension, IntoPlutusData, PlutusDataExtension,
};
use spectrum_cardano_lib::transaction::TransactionOutputExtension;
use spectrum_cardano_lib::types::TryFromPData;
use spectrum_cardano_lib::value::ValueExtension;
use spectrum_offchain::data::{Has, Stable};
use spectrum_offchain::ledger::{IntoLedger, TryFromLedger};

use crate::data::limit_swap::ClassicalOnChainLimitSwap;
use crate::data::operation_output::SwapOutput;
use crate::data::order::{Base, ClassicalOrder, PoolNft, Quote};
use crate::data::pair::order_canonical;
use crate::data::pool::{
    ApplyOrder, ApplyOrderError, CFMMPoolAction, ImmutablePoolUtxo, PoolAssetMapping, PoolBounds, Rx, Ry,
};
use crate::data::PoolId;
use crate::deployment::{DeployedScriptInfo, DeployedValidator, DeployedValidatorErased, RequiresValidator};
use crate::deployment::ProtocolValidator::DegenQuadraticPoolV1;
use crate::fees::FeeExtension;
use crate::pool_math::degen_quadratic_math::{A_DENOM, B_DENOM, degen_quadratic_output_amount};

pub struct DegenQuadraticPoolConfig {
    pub pool_nft: TaggedAssetClass<PoolNft>,
    pub asset_x: TaggedAssetClass<Rx>,
    pub asset_y: TaggedAssetClass<Ry>,
    pub a_num: u64,
    pub b_num: u64,
    pub ada_cup_thr: u64,
}

impl TryFromPData for DegenQuadraticPoolConfig {
    fn try_from_pd(data: PlutusData) -> Option<Self> {
        let mut cpd = data.into_constr_pd()?;
        let pool_nft = TaggedAssetClass::try_from_pd(cpd.take_field(0)?)?;
        let asset_x = TaggedAssetClass::try_from_pd(cpd.take_field(1)?)?;
        let asset_y = TaggedAssetClass::try_from_pd(cpd.take_field(2)?)?;
        let a_num = cpd.take_field(3)?.into_u64()?;
        let b_num = cpd.take_field(4)?.into_u64()?;
        let ada_cup_thr = cpd.take_field(5)?.into_u64()?;

        Some(Self {
            pool_nft,
            asset_x,
            asset_y,
            a_num,
            b_num,
            ada_cup_thr,
        })
    }
}

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub enum DegenQuadraticPoolVer {
    V1,
}

impl DegenQuadraticPoolVer {
    pub fn try_from_address<Ctx>(pool_addr: &Address, ctx: &Ctx) -> Option<DegenQuadraticPoolVer>
    where
        Ctx: Has<DeployedScriptInfo<{ DegenQuadraticPoolV1 as u8 }>> + Has<PoolBounds>,
    {
        let maybe_hash = pool_addr.payment_cred().and_then(|c| match c {
            StakeCredential::PubKey { .. } => None,
            StakeCredential::Script { hash, .. } => Some(hash),
        });
        if let Some(this_hash) = maybe_hash {
            if ctx
                .select::<DeployedScriptInfo<{ DegenQuadraticPoolV1 as u8 }>>()
                .script_hash
                == *this_hash
            {
                return Some(DegenQuadraticPoolVer::V1);
            }
        }
        None
    }
}

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub struct DegenQuadraticPool {
    pub id: PoolId,
    pub reserves_x: TaggedAmount<Rx>,
    pub reserves_y: TaggedAmount<Ry>,
    pub asset_x: TaggedAssetClass<Rx>,
    pub asset_y: TaggedAssetClass<Ry>,
    pub a_num: u64,
    pub b_num: u64,
    pub ada_cup_thr: u64,
    pub ver: DegenQuadraticPoolVer,
    pub marginal_cost: ExUnits,
    pub bounds: PoolBounds,
}

impl DegenQuadraticPool {
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
    fn create_redeemer(pool_in_idx: u64, pool_out_idx: u64) -> PlutusData {
        let self_ix_pd = PlutusData::Integer(BigInteger::from(pool_in_idx));
        let self_out_pd = PlutusData::Integer(BigInteger::from(pool_out_idx));
        let amm_action = PlutusData::ConstrPlutusData(ConstrPlutusData::new(0, Vec::from([])));

        PlutusData::ConstrPlutusData(ConstrPlutusData::new(
            0,
            Vec::from([self_ix_pd, self_out_pd, amm_action]),
        ))
    }
}

pub struct DegenQuadraticPoolRedeemer {
    pub pool_input_index: u64,
    pub pool_output_index: u64,
    pub action: CFMMPoolAction,
}

impl DegenQuadraticPoolRedeemer {
    pub fn to_plutus_data(self) -> PlutusData {
        DegenQuadraticPool::create_redeemer(self.pool_input_index, self.pool_output_index)
    }
}

pub trait AMMOps {
    fn output_amount(
        &self,
        base_asset: TaggedAssetClass<Base>,
        base_amount: TaggedAmount<Base>,
    ) -> TaggedAmount<Quote>;
}

impl AMMOps for DegenQuadraticPool {
    fn output_amount(
        &self,
        base_asset: TaggedAssetClass<Base>,
        base_amount: TaggedAmount<Base>,
    ) -> TaggedAmount<Quote> {
        degen_quadratic_output_amount(
            self.asset_x,
            self.reserves_y,
            base_asset,
            base_amount,
            self.a_num,
            self.b_num,
        )
    }
}

impl<Ctx> RequiresValidator<Ctx> for DegenQuadraticPool
where
    Ctx: Has<DeployedValidator<{ DegenQuadraticPoolV1 as u8 }>>,
{
    fn get_validator(&self, ctx: &Ctx) -> DeployedValidatorErased {
        match self.ver {
            DegenQuadraticPoolVer::V1 => ctx
                .select::<DeployedValidator<{ DegenQuadraticPoolV1 as u8 }>>()
                .erased(),
        }
    }
}

impl MakerBehavior for DegenQuadraticPool {
    fn swap(mut self, input: OnSide<u64>) -> Next<Self, Unit> {
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
        let (base_reserves, quote_reserves) = if x == base {
            (self.reserves_x.as_mut(), self.reserves_y.as_mut())
        } else {
            (self.reserves_y.as_mut(), self.reserves_x.as_mut())
        };
        match input {
            OnSide::Bid(input) => {
                // A user bid means that they wish to buy the base asset for the quote asset, hence
                // pool reserves of base decreases while reserves of quote increase.
                *quote_reserves += input;
                *base_reserves -= output;
            }
            OnSide::Ask(input) => {
                // User ask is the opposite; sell the base asset for the quote asset.
                *base_reserves += input;
                *quote_reserves -= output;
            }
        }
        Next::Succ(self)
    }
}

impl MarketMaker for crate::data::degen_quadratic_pool::DegenQuadraticPool {
    type U = ExUnits;

    fn static_price(&self) -> SpotPrice {
        let x = self.asset_x.untag();
        let y = self.asset_y.untag();
        let [base, _] = order_canonical(x, y);
        if x == base {
            AbsolutePrice::new_raw(
                (self.reserves_y.untag() * self.reserves_y.untag() * self.a_num) as u128 * B_DENOM
                    + A_DENOM * self.b_num as u128,
                A_DENOM * B_DENOM,
            )
            .into()
        } else {
            AbsolutePrice::new_unsafe(self.reserves_x.untag(), self.reserves_y.untag()).into()
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
        unimplemented!()
    }

    fn marginal_cost_hint(&self) -> Self::U {
        self.marginal_cost
    }

    fn is_active(&self) -> bool {
        unimplemented!()
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
}

impl Has<DegenQuadraticPoolVer> for DegenQuadraticPool {
    fn select<U: IsEqual<DegenQuadraticPoolVer>>(
        &self,
    ) -> crate::data::degen_quadratic_pool::DegenQuadraticPoolVer {
        self.ver
    }
}

impl Stable for crate::data::degen_quadratic_pool::DegenQuadraticPool {
    type StableId = PoolId;
    fn stable_id(&self) -> Self::StableId {
        self.id
    }
    fn is_quasi_permanent(&self) -> bool {
        true
    }
}

impl<Ctx> TryFromLedger<BabbageTransactionOutput, Ctx> for DegenQuadraticPool
where
    Ctx: Has<DeployedScriptInfo<{ DegenQuadraticPoolV1 as u8 }>> + Has<PoolBounds>,
{
    fn try_from_ledger(repr: &BabbageTransactionOutput, ctx: &Ctx) -> Option<Self> {
        if let Some(pool_ver) = DegenQuadraticPoolVer::try_from_address(repr.address(), ctx) {
            let value = repr.value();
            let pd = repr.datum().clone()?.into_pd()?;
            let bounds = ctx.select::<PoolBounds>();
            let marginal_cost = match pool_ver {
                DegenQuadraticPoolVer::V1 => {
                    ctx.select::<DeployedScriptInfo<{ DegenQuadraticPoolV1 as u8 }>>()
                        .marginal_cost
                }
            };
            match pool_ver {
                DegenQuadraticPoolVer::V1 => {
                    let conf = DegenQuadraticPoolConfig::try_from_pd(pd.clone())?;
                    return Some(DegenQuadraticPool {
                        id: PoolId::try_from(conf.pool_nft).ok()?,
                        reserves_x: TaggedAmount::new(value.amount_of(conf.asset_x.into())?),
                        reserves_y: TaggedAmount::new(value.amount_of(conf.asset_y.into())?),
                        asset_x: conf.asset_x,
                        asset_y: conf.asset_y,
                        a_num: conf.a_num,
                        b_num: conf.b_num,
                        ver: pool_ver,
                        marginal_cost,
                        bounds,
                        ada_cup_thr: 0,
                    });
                }
            };
        };
        None
    }
}

impl IntoLedger<TransactionOutput, ImmutablePoolUtxo> for DegenQuadraticPool {
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
        let (nft_lq, name_nft) = self.id.into();
        ma.set(nft_lq, name_nft.into(), 1);

        TransactionOutput::new_conway_format_tx_out(ConwayFormatTxOut {
            address: immut_pool.address,
            amount: Value::new(coins, ma),
            datum_option: immut_pool.datum_option,
            script_reference: immut_pool.script_reference,
            encodings: None,
        })
    }
}

pub fn unsafe_update_pd(data: &mut PlutusData, treasury_x: u64, treasury_y: u64) {
    let cpd = data.get_constr_pd_mut().unwrap();
    cpd.set_field(6, treasury_x.into_pd());
    cpd.set_field(7, treasury_y.into_pd());
}

impl ApplyOrder<ClassicalOnChainLimitSwap> for DegenQuadraticPool {
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

mod tests {
    use cml_crypto::ScriptHash;
    use rand::{Rng, SeedableRng};
    use rand::prelude::StdRng;
    use rand::seq::SliceRandom;

    use bloom_offchain::execution_engine::liquidity_book::core::Next;
    use bloom_offchain::execution_engine::liquidity_book::market_maker::MakerBehavior;
    use bloom_offchain::execution_engine::liquidity_book::side::OnSide;
    use spectrum_cardano_lib::{AssetClass, AssetName, TaggedAmount, TaggedAssetClass};
    use spectrum_cardano_lib::ex_units::ExUnits;

    use crate::data::degen_quadratic_pool::{DegenQuadraticPool, DegenQuadraticPoolVer};
    use crate::data::pool::PoolBounds;
    use crate::data::PoolId;
    use crate::pool_math::degen_quadratic_math::{A_DENOM, calculate_a_num, TOKEN_EMISSION};

    const LOVELACE: u64 = 1_000_000;
    const DEC: u64 = 1_000;

    fn gen_ada_token_pool(
        reserves_x: u64,
        reserves_y: u64,
        a_num: u64,
        b_num: u64,
        ada_thr: u64,
    ) -> DegenQuadraticPool {
        return DegenQuadraticPool {
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
            reserves_x: TaggedAmount::new(reserves_x),
            reserves_y: TaggedAmount::new(reserves_y),
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
            a_num,
            b_num,
            ver: DegenQuadraticPoolVer::V1,
            marginal_cost: ExUnits { mem: 100, steps: 100 },
            bounds: PoolBounds {
                min_n2t_lovelace: 10000000,
                min_t2t_lovelace: 10000000,
            },
            ada_cup_thr: ada_thr,
        };
    }

    #[test]
    fn deposit_redeem_consistency_test() {
        let mut rng = StdRng::seed_from_u64(42);
        for _ in 0..1_000 {
            let ada_cap: u64 = rng.gen_range(1_000..100_000) * LOVELACE;
            let a: u64 = calculate_a_num(&ada_cap);
            let to_dep_init: u64 = rng.gen_range(100_000..1_000_000_000);
            let to_dep: u64 = rng.gen_range(100_000..1_000_000_000);
            let b: u64 = 0;

            let pool = gen_ada_token_pool(0, TOKEN_EMISSION, a, b, ada_cap);

            let Next::Succ(pool0) = pool.swap(OnSide::Ask(to_dep_init)) else {
                panic!()
            };

            let Next::Succ(pool1) = pool0.swap(OnSide::Ask(to_dep)) else {
                panic!()
            };

            let y_rec = pool0.reserves_y.untag() - pool1.reserves_y.untag();

            let Next::Succ(pool2) = pool1.swap(OnSide::Bid(y_rec)) else {
                panic!()
            };

            let x_rec = pool1.reserves_x.untag() - pool2.reserves_x.untag();
            assert!((to_dep - x_rec) * DEC <= to_dep);
        }
    }

    #[test]
    fn full_flow_test() {
        let mut rng = StdRng::seed_from_u64(42);
        let min_redeem = 10u64;
        let b: u64 = 0;
        for _ in 0..3 {
            let ada_cap: u64 = rng.gen_range(1_000..100_000 * LOVELACE);
            let a: u64 = calculate_a_num(&ada_cap);
            let to_dep_init: u64 = rng.gen_range(LOVELACE..100 * LOVELACE);

            let pool = gen_ada_token_pool(0, TOKEN_EMISSION, a, b, ada_cap);

            let Next::Succ(pool) = pool.swap(OnSide::Ask(to_dep_init)) else {
                panic!()
            };
            let mut total_dep = pool.reserves_x.untag().clone();
            let mut reserves_y = pool.reserves_y.untag().clone();
            let mut last_price_num = 0;
            let mut n_operations = 0;
            while total_dep < ada_cap {
                let side = [0, 1, 0].choose(&mut rand::thread_rng()).unwrap();

                let pool_next = if *side == 0 {
                    let cap_remaining = ada_cap - pool.reserves_x.untag();
                    let in_amount: u64 = rng.gen_range(LOVELACE..ada_cap);
                    let in_amount = if in_amount < cap_remaining {
                        in_amount
                    } else {
                        cap_remaining
                    };
                    pool.swap(OnSide::Ask(in_amount))
                } else {
                    let total_supp = TOKEN_EMISSION - reserves_y;
                    let out_amount: u64 = rng.gen_range(min_redeem..total_supp / DEC);
                    let out_amount = if out_amount < total_supp {
                        out_amount
                    } else {
                        continue;
                    };
                    pool.swap(OnSide::Bid(out_amount))
                };
                let Next::Succ(pool) = pool_next else { panic!() };
                total_dep = pool.reserves_x.untag().clone();
                reserves_y = pool.reserves_y.untag().clone();
                last_price_num = pool.a_num as u128
                    * (TOKEN_EMISSION - reserves_y) as u128
                    * (TOKEN_EMISSION - reserves_y) as u128;
                n_operations += 1
            }
            assert_eq!(ada_cap, total_dep);
            assert!(last_price_num <= ada_cap as u128 * A_DENOM / reserves_y as u128)
        }
    }

    #[test]
    fn test_deposit_redeem_fixtures() {
        let min_utxo = 1_000;
        let ada_cap: u64 = 42_014 * LOVELACE;
        let a: u64 = calculate_a_num(&ada_cap);
        let b: u64 = 0;
        let to_dep: u64 = 123_010_079;
        let pool0 = gen_ada_token_pool(min_utxo, TOKEN_EMISSION, a, b, ada_cap);

        let Next::Succ(pool1) = pool0.swap(OnSide::Ask(to_dep)) else {
            panic!()
        };

        let y_rec = pool0.reserves_y.untag() - pool1.reserves_y.untag();

        let Next::Succ(pool2) = pool1.swap(OnSide::Bid(y_rec)) else {
            panic!()
        };

        let x_rec = pool1.reserves_x.untag() - pool2.reserves_x.untag();
        let y_real_delta = pool2.reserves_y.untag() - pool1.reserves_y.untag();

        assert_eq!(a, 298759111111);
        assert_eq!(y_rec, 107295192);
        assert_eq!(x_rec, 123010076);
        assert_eq!(y_real_delta, y_rec);
    }
}
