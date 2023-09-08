use cml_chain::address::Address;
use lazy_static::lazy_static;

lazy_static! {
    pub static ref POOL_ADDR_V1: Address =
        Address::from_bech32("addr1wx3937ykmlcaqxkf4z7stxpsfwfn4re7ncy48yu8vutcpxgg67me2").unwrap();
}

pub const CFMM_LP_FEE_DEN: u64 = 1000;
