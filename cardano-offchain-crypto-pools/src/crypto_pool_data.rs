use primitive_types::U512;

#[derive(Clone)]
pub struct PoolConfig {
    pub n_tradable_assets: U512,
    pub ampl_coeff: U512,
    pub swap_fee_num: U512,
    pub protocol_fee_num: U512,
    pub gamma_num: U512,
    pub fee_gamma_num: U512,
    pub fee_mid_num: U512,
    pub fee_out_num: U512,
}

#[derive(Clone)]
pub struct PoolStateData {
    pub price_scale: Vec<U512>,
    pub xcp_profit: U512,
    pub xcp_profit_real: U512,
}

#[derive(Clone)]
pub struct PoolReserves {
    pub tradable: Vec<U512>,
    pub lp_tokens: U512,
}
