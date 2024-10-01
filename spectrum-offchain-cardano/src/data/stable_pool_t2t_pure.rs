#[cfg(test)]
mod tests {
    use crate::constants::MAX_LQ_CAP;
    use crate::data::order::ClassicalOrder;
    use crate::data::order::OrderType::BalanceFn;
    use crate::data::pool::ApplyOrder;
    use crate::data::redeem::{ClassicalOnChainRedeem, Redeem};
    use crate::data::stable_pool_t2t::{StablePoolT2T, StablePoolT2TConfig, StablePoolT2TVer};
    use crate::data::{OnChainOrderId, PoolId};
    use crate::pool_math::stable_pool_t2t_math::{
        calculate_invariant, calculate_safe_price_ratio_x_y_swap,
    };
    use bloom_offchain::execution_engine::liquidity_book::core::Next;
    use bloom_offchain::execution_engine::liquidity_book::market_maker::{
        AvailableLiquidity, MakerBehavior, MarketMaker,
    };
    use bloom_offchain::execution_engine::liquidity_book::side::OnSide;
    use bloom_offchain::execution_engine::liquidity_book::side::OnSide::{Ask, Bid};
    use bloom_offchain::execution_engine::liquidity_book::types::AbsolutePrice;
    use cml_chain::plutus::PlutusData;
    use cml_chain::Deserialize;
    use cml_crypto::{Ed25519KeyHash, ScriptHash, TransactionHash};
    use num_rational::Ratio;
    use num_traits::ToPrimitive;
    use primitive_types::U512;
    use spectrum_cardano_lib::ex_units::ExUnits;
    use spectrum_cardano_lib::types::TryFromPData;
    use spectrum_cardano_lib::{AssetClass, AssetName, OutputRef, TaggedAmount, TaggedAssetClass, Token};

    const DATUM_SAMPLE: &str = "d8799fd8799f581c7dbe6f0c7849e2dae806cd4681910bfe1bbc0d5fd4e370e8e2f7bd4a436e6674ff190c80d8799f4040ffd8799f581c4b3459fd18a1dbabe207cd19c9951a9fac9f5c0f9c384e3d97efba26457465737443ff0000d8799f581c6abe65f6adc8301ff4dbfcfcec1a187075639d21f85cae3c1cf2a060426c71ffd87980d879801a000186820a581c4b3459fd18a1dbabe207cd19c9951a9fac9f5c0f9c384e3d97efba26581c4b3459fd18a1dbabe207cd19c9951a9fac9f5c0f9c384e3d97efba260000ff";

    fn gen_ada_token_pool(
        reserves_x: u64,
        x_decimals: u32,
        reserves_y: u64,
        y_decimals: u32,
        lp_fee_x: u64,
        lp_fee_y: u64,
        treasury_fee: u64,
        treasury_x: u64,
        treasury_y: u64,
        an2n: u64,
    ) -> StablePoolT2T {
        let reserves_x = reserves_x;
        let reserves_y = reserves_y;
        let (multiplier_x, multiplier_y) = if (x_decimals > y_decimals) {
            (1, 10_u32.pow(x_decimals - y_decimals))
        } else if (x_decimals < y_decimals) {
            (10_u32.pow(y_decimals - x_decimals), 1)
        } else {
            (1, 1)
        };
        let inv_before = calculate_invariant(
            &U512::from((reserves_x - treasury_x) * multiplier_x as u64),
            &U512::from((reserves_y - treasury_y) * multiplier_y as u64),
            &U512::from(an2n),
        )
            .unwrap();
        let liquidity = MAX_LQ_CAP - inv_before.as_u64();

        return StablePoolT2T {
            id: PoolId::from(Token(
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
            an2n: an2n, // constant
            reserves_x: TaggedAmount::new(reserves_x),
            multiplier_x: multiplier_x as u64,
            reserves_y: TaggedAmount::new(reserves_y),
            multiplier_y: multiplier_y as u64,
            liquidity: TaggedAmount::new(liquidity),
            asset_x: TaggedAssetClass::new(AssetClass::Native),
            asset_y: TaggedAssetClass::new(AssetClass::Token(Token(
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
            asset_lq: TaggedAssetClass::new(AssetClass::Token(Token(
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
            lp_fee_x: Ratio::new_raw(lp_fee_x, 100000),
            lp_fee_y: Ratio::new_raw(lp_fee_y, 100000),
            treasury_fee: Ratio::new_raw(treasury_fee, 100000),
            treasury_x: TaggedAmount::new(treasury_x),
            treasury_y: TaggedAmount::new(treasury_y),
            ver: StablePoolT2TVer::V1,
            marginal_cost: ExUnits {
                mem: 120000000,
                steps: 100000000000,
            },
        };
    }

    #[test]
    fn parse_stable_pool_t2t_datum() {
        let pd = PlutusData::from_cbor_bytes(&*hex::decode(DATUM_SAMPLE).unwrap()).unwrap();
        let maybe_conf = StablePoolT2TConfig::try_from_pd(pd);
        assert!(maybe_conf.is_some())
    }

    #[test]
    fn swap() {
        // Swap to min decimals;
        let pool = gen_ada_token_pool(
            475000220,
            6,
            343088,
            3,
            20000,
            20000,
            50000,
            220000220,
            88088,
            300 * 16,
        );

        let Next::Succ(result) = pool.swap(OnSide::Bid(390088 - 343088)) else {
            panic!()
        };

        assert_eq!(result.reserves_x.untag(), 460904695);
        assert_eq!(result.reserves_y.untag(), 390088);
        assert_eq!(result.treasury_x.untag(), 243492763);
        assert_eq!(result.treasury_y.untag(), 88088);

        // Swap to max decimals;
        let pool = gen_ada_token_pool(
            343088,
            3,
            475000220,
            6,
            20000,
            20000,
            50000,
            88088,
            220000220,
            300 * 16,
        );

        let Next::Succ(result) = pool.swap(OnSide::Ask(390088 - 343088)) else {
            panic!()
        };

        assert_eq!(result.reserves_x.untag(), 390088);
        assert_eq!(result.reserves_y.untag(), 460904695);
        assert_eq!(result.treasury_x.untag(), 88088);
        assert_eq!(result.treasury_y.untag(), 243492763);

        // Uniform swap;
        let pool = gen_ada_token_pool(100000, 1, 100000, 1, 2000, 2000, 5000, 0, 0, 300 * 16);

        let Next::Succ(result) = pool.swap(OnSide::Ask(1000)) else {
            panic!()
        };

        assert_eq!(result.reserves_x.untag(), 101000);
        assert_eq!(result.reserves_y.untag(), 99071);
        assert_eq!(result.treasury_x.untag(), 0);
        assert_eq!(result.treasury_y.untag(), 50);

        // Some swap;
        let pool = gen_ada_token_pool(100100000, 1, 99900201, 1, 100, 100, 100, 0, 100, 300 * 16);

        let Next::Succ(result) = pool.swap(OnSide::Ask(100000)) else {
            panic!()
        };

        assert_eq!(result.reserves_x.untag(), 100200000);
        assert_eq!(result.reserves_y.untag(), 99800402);
        assert_eq!(result.treasury_x.untag(), 0);
        assert_eq!(result.treasury_y.untag(), 200);

        // Uniform swap;
        let pool = gen_ada_token_pool(108500000, 1, 108500000, 1, 100, 100, 100, 0, 0, 200 * 16);

        let Next::Succ(result) = pool.swap(OnSide::Ask(100000000)) else {
            panic!()
        };

        assert_eq!(result.reserves_x.untag(), 208500000);
        assert_eq!(result.reserves_y.untag(), 9990108);
        assert_eq!(result.treasury_x.untag(), 0);
        assert_eq!(result.treasury_y.untag(), 98707);
    }

    #[test]
    fn deposit_redeemer_test() {
        let pool = gen_ada_token_pool(
            1_000_000_000,
            9,
            1_000_000_000,
            9,
            99000,
            99000,
            100,
            0,
            0,
            300 * 16,
        );

        const TX: &str = "6c038a69587061acd5611507e68b1fd3a7e7d189367b7853f3bb5079a118b880";
        const IX: u64 = 1;

        let test_order: ClassicalOnChainRedeem = ClassicalOrder {
            id: OnChainOrderId(OutputRef::new(TransactionHash::from_hex(TX).unwrap(), IX)),
            pool_id: pool.id,
            order: Redeem {
                pool_nft: pool.id,
                token_x: pool.asset_x,
                token_y: pool.asset_y,
                token_lq: pool.asset_lq,
                token_lq_amount: TaggedAmount::new(1900727),
                ex_fee: 1500000,
                reward_pkh: Ed25519KeyHash::from([0u8; 28]),
                reward_stake_pkh: None,
                collateral_ada: 3000000,
                order_type: BalanceFn,
            },
        };

        let test = pool.apply_order(test_order);

        assert_eq!(1, 1)
    }

    #[test]
    fn available_liquidity_test() {
        let pool = gen_ada_token_pool(3730031816494, 1, 3701037440628, 1, 100, 100, 0, 0, 0, 200 * 16);

        let worst_price = AbsolutePrice::new(87, 100).unwrap();
        let Some(AvailableLiquidity {
                     input: inp,
                     output: out,
                 }) = pool.available_liquidity_on_side(Ask(worst_price))
            else {
                !panic!()
            };
        assert_eq!(inp, 4216348326046);
        assert_eq!(out, 3668223043660);

        let pool = gen_ada_token_pool(3701037440628, 1, 3730031816494, 1, 100, 100, 0, 0, 0, 200 * 16);

        let worst_price = AbsolutePrice::new(100, 87).unwrap();
        let Some(AvailableLiquidity {
                     input: inp,
                     output: out,
                 }) = pool.available_liquidity_on_side(Bid(worst_price))
            else {
                !panic!()
            };
        assert_eq!(inp, 4216348326046);
        assert_eq!(out, 3668223043660)
    }

    #[test]
    fn simple_swap() {
        // Swap to min decimals;
        let pool = gen_ada_token_pool(
            1_000_000_000_000,
            0,
            1_000_000_000_000,
            0,
            0,
            0,
            0,
            0,
            0,
            200 * 16,
        );

        let Next::Succ(result) = pool.swap(OnSide::Ask(1_000_000)) else {
            panic!()
        };
        let y_rec = pool.reserves_y.untag() - result.reserves_y.untag();
        assert_eq!(y_rec, 999999);
        assert_eq!(result.reserves_x.untag(), 1000001000000);
        assert_eq!(result.reserves_y.untag(), 999999000001);
        assert_eq!(result.treasury_x.untag(), 0);
        assert_eq!(result.treasury_y.untag(), 0);
    }
}
