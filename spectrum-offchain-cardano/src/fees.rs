use num_rational::Ratio;

pub type FeePerOutput = Ratio<u128>;

pub trait FeeExtension {
    fn linear_fee(self, output: u64) -> u64;
}

impl FeeExtension for FeePerOutput {
    fn linear_fee(self, output: u64) -> u64 {
        u64::try_from((self * output as u128).to_integer()).unwrap()
    }
}
