#[derive(Debug, Clone, PartialEq)]
pub enum PoolMathError {
    Divizio,
    OrderUtxoIsSpent,
    UnknownError { info: String },
}