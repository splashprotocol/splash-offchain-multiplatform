use cardano_explorer::CardanoNetwork;
use spectrum_offchain_cardano::deployment::{
    DeployedScriptInfo, DeployedValidator, DeployedValidatorRef, ProtocolValidator,
};

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SnekDeployedValidators {
    pub instant_order_witness: DeployedValidatorRef,
    pub instant_order: DeployedValidatorRef,
    pub degen_fn_pool_v1: DeployedValidatorRef,
}

#[derive(Debug, Clone)]
pub struct SnekProtocolDeployment {
    pub instant_order_witness: DeployedValidator<{ ProtocolValidator::InstantOrderWitnessV1 as u8 }>,
    pub instant_order: DeployedValidator<{ ProtocolValidator::InstantOrderV1 as u8 }>,
    pub quadratic_pool_v1: DeployedValidator<{ ProtocolValidator::DegenQuadraticPoolV1 as u8 }>,
}

impl SnekProtocolDeployment {
    pub async fn unsafe_pull<Net: CardanoNetwork>(
        validators: SnekDeployedValidators,
        explorer: &Net,
    ) -> Self {
        Self {
            instant_order_witness: DeployedValidator::unsafe_pull(validators.instant_order_witness, explorer)
                .await,
            instant_order: DeployedValidator::unsafe_pull(validators.instant_order, explorer).await,
            quadratic_pool_v1: DeployedValidator::unsafe_pull(validators.degen_fn_pool_v1, explorer).await,
        }
    }
}

#[derive(Debug, Copy, Clone)]
pub struct SnekProtocolScriptHashes {
    pub instant_order_witness: DeployedScriptInfo<{ ProtocolValidator::InstantOrderWitnessV1 as u8 }>,
    pub instant_order: DeployedScriptInfo<{ ProtocolValidator::InstantOrderV1 as u8 }>,
    pub degen_fn_pool_v1: DeployedScriptInfo<{ ProtocolValidator::DegenQuadraticPoolV1 as u8 }>,
}

impl From<&SnekProtocolDeployment> for SnekProtocolScriptHashes {
    fn from(deployment: &SnekProtocolDeployment) -> Self {
        Self {
            instant_order_witness: From::from(&deployment.instant_order_witness),
            instant_order: From::from(&deployment.instant_order),
            degen_fn_pool_v1: From::from(&deployment.quadratic_pool_v1),
        }
    }
}
