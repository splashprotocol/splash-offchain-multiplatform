use std::ops::Div;

use bigdecimal::num_bigint::ToBigInt;
use bigdecimal::BigDecimal;
use log::info;
use num_traits::{Pow, ToPrimitive};

use spectrum_cardano_lib::{TaggedAmount, TaggedAssetClass};

use crate::data::order::{Base, Quote};

pub const MAX_ALLOWED_ADA_EXTRAS_PERCENTILE: u64 = 1;
pub const FULL_PERCENTILE: u64 = 1000;

pub const MIN_ADA: u64 = 3_000_000;

pub const A_DENOM: u128 = 1_000_000_000_000_000_000_000_000_000;
pub const B_DENOM: u128 = 1_000_000;
pub const TOKEN_EMISSION: u64 = 1_000_000_000;

pub fn degen_quadratic_output_amount<X, Y>(
    asset_x: TaggedAssetClass<X>,
    reserves_y: TaggedAmount<Y>,
    base_asset: TaggedAssetClass<Base>,
    base_amount: TaggedAmount<Base>,
    a_num: u64,
    b_num: u64,
    accumulated_x_fee: TaggedAmount<X>,
) -> TaggedAmount<Quote> {
    let token_supply0 = BigDecimal::from(TOKEN_EMISSION - reserves_y.untag());
    let available_base_amount = BigDecimal::from(base_amount.untag());
    let a_denom = BigDecimal::from(A_DENOM);
    let b_denom = BigDecimal::from(B_DENOM);
    let a_num = BigDecimal::from(a_num);
    let b_num = BigDecimal::from(b_num);
    let const_3 = BigDecimal::from(3);
    let const_27 = BigDecimal::from(27);
    let const_54 = BigDecimal::from(54);
    let const_81 = BigDecimal::from(81);
    let const_2916 = BigDecimal::from(2916);

    let token_supply0_x3 = token_supply0.clone() * token_supply0.clone() * token_supply0.clone();
    let quote_amount = if base_asset.untag() == asset_x.untag() {
        let a_denom_const_81_a_num_x2 = a_denom.clone() * const_81 * a_num.clone() * a_num.clone();
        let a_num_x3 = a_num.clone() * a_num.clone() * a_num.clone();
        let a_denom_x3 = a_denom.clone() * a_denom.clone() * a_denom.clone();
        let brackets = (b_denom.clone() * const_27 * a_num_x3.clone() * token_supply0_x3.clone()
            + a_denom_const_81_a_num_x2.clone() * b_num.clone() * token_supply0.clone()
            + b_denom.clone() * a_denom_const_81_a_num_x2.clone() * available_base_amount.clone())
        .div(b_denom.clone() * a_denom_x3.clone());
        let b_num_x3 = b_num.clone() * b_num.clone() * b_num.clone();
        let b_denom_x3 = b_denom.clone() * b_denom.clone() * b_denom.clone();
        let brackets_comb = (brackets.clone()
            + (const_2916.clone() * a_num_x3.clone() * b_num_x3.clone()
                + a_denom_x3.clone() * b_denom_x3.clone() * brackets.clone() * brackets.clone())
            .div(a_denom_x3.clone() * b_denom_x3.clone())
            .sqrt()
            .unwrap())
        .cbrt();
        let token_supply1 = (a_denom.clone() * brackets_comb.clone())
            .div((const_54.clone() * a_num_x3.clone()).cbrt())
            - (const_54.clone() * b_num_x3.clone())
                .cbrt()
                .div(brackets_comb.clone() * b_denom.clone());
        token_supply1 - token_supply0
    } else {
        let token_supply1 = token_supply0.clone() - available_base_amount;

        a_num.clone()
            * (token_supply0_x3.clone()
                - token_supply1.clone() * token_supply1.clone() * token_supply1.clone())
            .div(const_3 * a_denom.clone())
            + b_num * (token_supply0.clone() - token_supply1.clone()).div(b_denom.clone())
    };
    TaggedAmount::new(quote_amount.to_u64().unwrap_or_else(|| 0u64).into())
}

pub fn calculate_a_num(ada_cap_thr: &u64) -> u64 {
    let lovelace = 1_000_000u64;
    let m = TOKEN_EMISSION.to_bigint().unwrap();
    let m_x3 = m.clone() * m.clone() * m.clone();
    let denom = A_DENOM.to_bigint().unwrap();
    let ada_cap = (*ada_cap_thr).to_bigint().unwrap();

    let mut a_num =
        64.to_bigint().unwrap() * denom.clone() * ada_cap.clone() / (9.to_bigint().unwrap() * m_x3.clone());
    let mut amm_price_ok = false;
    let mut ada_cap_iter = (*ada_cap_thr).clone();
    let mut counter = 0;
    while !amm_price_ok && counter < 255 {
        ada_cap_iter -= lovelace;
        a_num = 64.to_bigint().unwrap() * denom.clone() * ada_cap_iter.to_bigint().unwrap()
            / (9.to_bigint().unwrap() * m_x3.clone());
        let final_supply =
            (3.to_bigint().unwrap() * ada_cap.clone() * denom.clone() / a_num.clone()).nth_root(3);
        let p_curve_final_num = a_num.clone() * final_supply.clone() * final_supply.clone();
        let p_amm_initial_num = ada_cap.clone() * denom.clone() / (m.clone() - final_supply.clone());
        amm_price_ok = p_curve_final_num <= p_amm_initial_num;
        counter += 1;
    }
    *a_num.to_u64_digits().1.first().unwrap()
}

mod tests {
    use crate::pool_math::degen_quadratic_math::calculate_a_num;

    #[test]
    fn calc_a_num_test() {
        let ada_cap = (42_069_000_000u64) * 75 / 100;
        let a_num = calculate_a_num(&ada_cap);
        assert_eq!(a_num, 224360888888)
    }
}
