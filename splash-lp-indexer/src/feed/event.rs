use crate::account::AccountInPool;
use cml_chain::certs::Credential;
use serde::{Deserialize, Serialize};
use spectrum_offchain_cardano::data::PoolId;

#[derive(Debug, Serialize, Deserialize)]
pub struct ExportAccountEvent {
    pub account_cred: Credential,
    pub pool_id: PoolId,
    pub update: AccountInPool,
}

#[cfg(test)]
mod tests {
    use crate::account::AccountInPool;
    use crate::feed::event::ExportAccountEvent;
    use cml_chain::certs::Credential;
    use cml_crypto::Ed25519KeyHash;
    use spectrum_offchain_cardano::data::PoolId;

    #[test]
    fn json_sample() {
        let sample = ExportAccountEvent {
            account_cred: Credential::new_pub_key(Ed25519KeyHash::from([0u8; 28])),
            pool_id: PoolId::random(),
            update: AccountInPool::new(1, true),
        };
        println!("{}", serde_json::to_string(&sample).unwrap());
    }
}
