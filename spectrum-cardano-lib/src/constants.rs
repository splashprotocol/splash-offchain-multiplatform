use cml_chain::PolicyId;
use lazy_static::lazy_static;

lazy_static! {
    pub static ref NATIVE_POLICY_ID: PolicyId = PolicyId::from([0u8; 28]);
}
