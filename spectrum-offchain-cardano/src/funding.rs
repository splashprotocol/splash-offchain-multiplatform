use std::fmt::{Debug, Display, Formatter};
use std::ops::Index;

use cml_chain::address::Address;
use derive_more::From;

#[derive(Clone, From)]
pub struct FundingAddresses<const N: usize>([Address; N]);

impl<const N: usize> Display for FundingAddresses<N> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        for addr in &self.0 {
            f.write_str(addr.to_bech32(None).unwrap().as_str())?;
            f.write_str(", ")?
        }
        Ok(())
    }
}

impl<const N: usize> Index<usize> for FundingAddresses<N> {
    type Output = Address;
    fn index(&self, index: usize) -> &Self::Output {
        self.0.index(index)
    }
}

impl<const N: usize> FundingAddresses<N> {
    pub fn partition_by_address(&self, address: &Address) -> Option<usize> {
        self.0.iter().position(|e| e == address)
    }
}
