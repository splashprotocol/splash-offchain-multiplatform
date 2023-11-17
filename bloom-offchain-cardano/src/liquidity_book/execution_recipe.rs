use spectrum_offchain_cardano::data::limit_swap::ClassicalOnChainLimitSwap;
use spectrum_offchain_cardano::data::pool::CFMMPool;
use spectrum_offchain_cardano::data::OnChain;

/// Instruction to execute order against a pool simply.
#[derive(Debug, Clone)]
pub struct OrderToPool<Pool, Order> {
    pub pool: OnChain<Pool>,
    pub order: OnChain<Order>,
}

/// Instruction to execute ask orders against bid orders.
#[derive(Debug, Clone)]
pub struct OrdersToOrders<O1, O2> {
    pub asks: Vec<OnChain<O1>>,
    pub bids: Vec<OnChain<O2>>,
}

/// Instruction to execute ask orders against bid orders and fill the remaining part from a pool.
#[derive(Debug, Clone)]
pub struct OrdersToOrdersAndPool<O1, O2, Pool> {
    pub asks: Vec<OnChain<O1>>,
    pub bids: Vec<OnChain<O2>>,
    pub pool: OnChain<Pool>,
}

/// Matchmaking instruction.
#[derive(Debug, Clone)]
pub enum ExecutionRecipe {
    OrderToCFMMPool(OrderToPool<CFMMPool, ClassicalOnChainLimitSwap>),
}
