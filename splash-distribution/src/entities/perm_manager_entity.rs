use cml_chain::transaction::TransactionOutput;
use log::info;
use serde::{Deserialize, Serialize};
use spectrum_cardano_lib::plutus_data::DatumExtension;
use spectrum_cardano_lib::transaction::TransactionOutputExtension;
use spectrum_cardano_lib::types::TryFromPData;
use spectrum_cardano_lib::OutputRef;
use spectrum_offchain::domain::Has;
use spectrum_offchain::ledger::TryFromLedger;
use spectrum_offchain_cardano::deployment::{test_address, DeployedScriptInfo};
use splash_dao_offchain::constants::DEFAULT_AUTH_TOKEN_NAME;
use splash_dao_offchain::deployment::ProtocolValidator;
use splash_dao_offchain::entities::onchain::permission_manager::{PermManager, PermManagerDatum};
use splash_dao_offchain::entities::{HasStatus, Snapshot};
use splash_dao_offchain::protocol_config::PermManagerAuthPolicy;
use splash_dao_offchain::routines::TimedOutputRef;

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq, Debug)]
pub struct PermManagerEntity {
    perm_manager: PermManager,
    status: PermManagerEntityStatus,
}

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq, Debug)]
pub enum PermManagerEntityStatus {
    Free,
    SFWithdraw,
}

impl HasStatus for PermManagerEntity {
    type Status = PermManagerEntityStatus;

    fn get_status(self) -> Self::Status {
        self.status
    }
}

impl<Ctx> TryFromLedger<TransactionOutput, Ctx> for PermManagerEntity
where
    Ctx: Has<PermManagerAuthPolicy>
        + Has<DeployedScriptInfo<{ ProtocolValidator::PermManager as u8 }>>,
{
    fn try_from_ledger(repr: &TransactionOutput, ctx: &Ctx) -> Option<Self> {
        //info!("Testing output with address {} against PermManagerEntity", repr.address().to_bech32(None).unwrap());
        if test_address(repr.address(), ctx) {
            let perm_manager_auth_policy = ctx.select::<PermManagerAuthPolicy>().0;
            let auth_token_cml_asset_name =
                cml_chain::assets::AssetName::new(DEFAULT_AUTH_TOKEN_NAME.to_be_bytes().to_vec()).unwrap();
            let auth_token_qty = repr
                .value()
                .multiasset
                .get(&perm_manager_auth_policy, &auth_token_cml_asset_name)?;
            if auth_token_qty == 1 {
                let datum = repr.datum()?;
                let perm_manager_datum = datum
                    .into_pd()
                    .map(|pd| PermManagerDatum::try_from_pd(pd).unwrap())?;
                let perm_manager = PermManager {
                    datum: perm_manager_datum,
                };

                return Some(PermManagerEntity {
                    perm_manager,
                    status: PermManagerEntityStatus::Free,
                });
            }
        }
        None
    }
}
