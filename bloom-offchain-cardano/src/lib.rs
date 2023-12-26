use std::fmt::{Display, Formatter};

use spectrum_cardano_lib::Token;

pub mod event_sink;
pub mod execution_engine;
pub mod operator_address;
pub mod orders;
pub mod pools;

#[derive(Debug, Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct PairId(Token, Token);

impl Display for PairId {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(format!("{}:{}", self.0.0, self.1.0).as_str())
    }
}
