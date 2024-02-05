use primitive_types::U512;

pub fn nonzero_prod(vec: &Vec<U512>) -> U512 {
    let zero = U512::from(0);
    vec.iter()
        .copied()
        .reduce(|x, y| {
            if x * y != zero {
                x * y
            } else if x == zero {
                y
            } else {
                x
            }
        })
        .unwrap()
}