use cml_chain::PolicyId;
use rand::Rng;

pub mod babbage;
pub mod pool;

fn gen_policy_id() -> PolicyId {
    let mut rng = rand::thread_rng();
    let mut bytes = [0u8; 28];
    rng.fill(&mut bytes[..]);
    PolicyId::from(bytes)
}
