use pallas_crypto::hash::Hash;
use pallas_primitives::babbage;

#[derive(Debug, Clone)]
pub struct Block {
    pub hash: Hash<32>,
    pub prev_hash: Hash<32>,
    pub slot_num: u64,
    pub txs: Vec<babbage::Tx>,
}