use crate::account::AccountPosition;
use cml_chain::certs::Credential;
use serde::{Deserialize, Serialize};
use spectrum_offchain_cardano::data::PoolId;
use splash_yf_offchain::Epoch;

#[derive(Debug, Serialize, Deserialize)]
pub struct ExportAccountPositionEvent {
    pub account_cred: Credential,
    pub pool_id: PoolId,
    pub epoch: Epoch,
    pub update: AccountPosition,
}

#[cfg(test)]
mod tests {
    use crate::account::AccountPosition;
    use crate::feed::event::ExportAccountPositionEvent;
    use cml_chain::certs::Credential;
    use cml_crypto::Ed25519KeyHash;
    use spectrum_offchain_cardano::data::PoolId;
    use splash_yf_offchain::Epoch;

    #[test]
    fn json_sample() {
        let sample = ExportAccountPositionEvent {
            account_cred: Credential::new_pub_key(Ed25519KeyHash::from([0u8; 28])),
            pool_id: PoolId::random(),
            epoch: Epoch::from(64),
            update: AccountPosition::new(1),
        };
        println!("{}", serde_json::to_string(&sample).unwrap());
    }
}
