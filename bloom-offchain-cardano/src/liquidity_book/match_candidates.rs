use spectrum_offchain_cardano::data::pool::CFMMPool;
use spectrum_offchain_cardano::data::OnChain;

#[derive(Debug, Clone)]
pub enum LiquiditySrc {
    CFMMPool(OnChain<CFMMPool>),
}

#[derive(Debug, Clone)]
pub struct MatchCandidates {
    pub asks: Vec<LiquiditySrc>,
    pub bids: Vec<LiquiditySrc>,
}
