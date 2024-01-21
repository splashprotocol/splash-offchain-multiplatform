use primitive_types::U512;

pub fn calculate_x_cp(n: &u32, nn: &U512, dn: &U512, price_vector: &Vec<U512>) -> U512 {
    let unit = U512::from(1);
    let n_calc = U512::from(*n);
    let p = price_vector
        .iter()
        .copied()
        .reduce(|a, b| if a != U512::from(0) { a * b } else { b })
        .unwrap();
    let c = dn / (nn * p);
    let mut abs_error = p;
    let x_cp_init = price_vector.iter().fold(unit, |a, b| a.max(*b));

    let mut x_cp = x_cp_init;

    while abs_error > unit {
        let x_cp_previous = x_cp;
        let x_cp_previous_n1 = vec![x_cp_previous; usize::try_from(n - 1).unwrap()]
            .iter()
            .copied()
            .reduce(|x, y| x * y)
            .unwrap();

        let x_cp_n = x_cp_previous_n1 * x_cp_previous;
        let f_value = if x_cp_n > c { x_cp_n - c } else { c - x_cp };
        x_cp = if x_cp_n > c {
            x_cp_previous - f_value / (n_calc * x_cp_previous_n1)
        } else {
            x_cp_previous + f_value / (n_calc * x_cp_previous_n1)
        };
        abs_error = if x_cp >= x_cp_previous {
            x_cp - x_cp_previous
        } else {
            x_cp_previous - x_cp
        };
    }
    x_cp
}

#[cfg(test)]
mod test {
    use crate::repegging_utils::calculate_x_cp;
    use primitive_types::U512;

    #[test]
    fn calculate_x_cp_test() {
        let n = 3u32;
        let nn = U512::from(n.pow(n));
        let price_vector = vec![U512::from(2u32), U512::from(2124u32), U512::from(7542221u32)];
        let d = U512::from(u64::MAX);
        let dn = vec![d; usize::try_from(n).unwrap()]
            .iter()
            .copied()
            .reduce(|x, y| x * y)
            .unwrap();
        let x_cp = calculate_x_cp(&n, &nn, &dn, &price_vector);
        assert_eq!(x_cp, U512::from(1935993435902969u64));
    }
}
