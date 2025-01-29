use cardano_explorer::CardanoNetwork;
use spectrum_offchain_cardano::deployment::{
    DeployedScriptInfo, DeployedValidator, DeployedValidatorRef, DeployedValidators, ProtocolValidator,
};

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SnekDeployedValidators {
    pub limit_order_witness: DeployedValidatorRef,
    pub limit_order: DeployedValidatorRef,
    pub degen_fn_pool_v1: DeployedValidatorRef,
}

#[derive(Debug, Clone)]
pub struct SnekProtocolDeployment {
    pub limit_order_witness: DeployedValidator<{ ProtocolValidator::LimitOrderWitnessV1 as u8 }>,
    pub limit_order: DeployedValidator<{ ProtocolValidator::LimitOrderV1 as u8 }>,
    pub degen_fn_pool_v1: DeployedValidator<{ ProtocolValidator::DegenQuadraticPoolV1 as u8 }>,
}

impl SnekProtocolDeployment {
    pub async fn unsafe_pull(validators: SnekDeployedValidators, explorer: &Box<dyn CardanoNetwork>) -> Self {
        Self {
            limit_order_witness: DeployedValidator::unsafe_pull(validators.limit_order_witness, explorer)
                .await,
            limit_order: DeployedValidator::unsafe_pull(validators.limit_order, explorer).await,
            degen_fn_pool_v1: DeployedValidator::unsafe_pull(validators.degen_fn_pool_v1, explorer).await,
        }
    }
}

#[derive(Debug, Copy, Clone)]
pub struct SnekProtocolScriptHashes {
    pub limit_order_witness: DeployedScriptInfo<{ ProtocolValidator::LimitOrderWitnessV1 as u8 }>,
    pub limit_order: DeployedScriptInfo<{ ProtocolValidator::LimitOrderV1 as u8 }>,
    pub degen_fn_pool_v1: DeployedScriptInfo<{ ProtocolValidator::DegenQuadraticPoolV1 as u8 }>,
}

impl From<&SnekProtocolDeployment> for SnekProtocolScriptHashes {
    fn from(deployment: &SnekProtocolDeployment) -> Self {
        Self {
            limit_order_witness: From::from(&deployment.limit_order_witness),
            limit_order: From::from(&deployment.limit_order),
            degen_fn_pool_v1: From::from(&deployment.degen_fn_pool_v1),
        }
    }
}
