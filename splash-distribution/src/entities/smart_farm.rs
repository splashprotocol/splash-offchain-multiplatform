use cml_chain::assets::AssetName;
use cml_chain::transaction::TransactionOutput;
use derive_more::Display;
use log::info;
use serde::{Deserialize, Serialize};
use spectrum_cardano_lib::plutus_data::DatumExtension;
use spectrum_cardano_lib::transaction::TransactionOutputExtension;
use spectrum_cardano_lib::types::TryFromPData;
use spectrum_cardano_lib::OutputRef;
use spectrum_offchain::domain::{Has, Stable};
use spectrum_offchain::ledger::TryFromLedger;
use spectrum_offchain_cardano::deployment::{test_address, DeployedScriptInfo};
use splash_dao_offchain::constants::{SPLASH_NAME, WPOLL_VOTE_ORDER_MIN_LOVELACES};
use splash_dao_offchain::deployment::ProtocolValidator;
use splash_dao_offchain::entities::onchain::smart_farm::{FarmId, SmartFarmConfig, SmartFarmSnapshot};
use splash_dao_offchain::entities::onchain::wpoll_vote_order::WPollVoteState;
use splash_dao_offchain::entities::{HasStatus, Snapshot};
use splash_dao_offchain::protocol_config::{FarmAuthPolicy, PermManagerAuthPolicy, SplashPolicy};
use splash_dao_offchain::routines::TimedOutputRef;

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq, Eq)]
pub struct SmartFarm {
    pub id: FarmId,
    pub status: SmartFarmStatus,
    pub splash_qty: u64,
    pub ada_qty: u64,
}

pub type DistributorSmartFarmSnapshot = Snapshot<SmartFarm, OutputRef>;

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq, Eq, Display)]
pub enum SmartFarmStatus {
    Free,
    SFWithdrawInProgress,
}

impl HasStatus for SmartFarm {
    type Status = SmartFarmStatus;

    fn get_status(self) -> Self::Status {
        self.status
    }
}

impl<C> TryFromLedger<TransactionOutput, C> for SmartFarm
where
    C: Has<PermManagerAuthPolicy>
        + Has<FarmAuthPolicy>
        + Has<SplashPolicy>
        + Has<DeployedScriptInfo<{ ProtocolValidator::SmartFarm as u8 }>>,
{
    fn try_from_ledger(repr: &TransactionOutput, ctx: &C) -> Option<Self> {
        //info!("Testing output with address {} against smart farm parser", repr.address().to_bech32(None).unwrap());
        if test_address(repr.address(), ctx) {
            let conf = SmartFarmConfig::try_from_pd(repr.datum()?.into_pd()?)?;

            let splash_asset_name = AssetName::try_from(SPLASH_NAME).unwrap();

            let splash_qty = repr
                .value()
                .multiasset
                .get(&ctx.select::<SplashPolicy>().0, &splash_asset_name)?;

            let ada_qty = repr.value().coin;

            for (policy_id, by_names) in repr.value().multiasset.iter() {
                let farm_auth_policy = ctx.select::<FarmAuthPolicy>().0;
                let verify_condition = *policy_id == farm_auth_policy && by_names.len() == 1;
                //info!("Testing values from multiasset value. Current policy_id is {}, by_names.len(): {}. Result is: {}", policy_id.to_hex(), by_names.len(), verify_condition);
                if verify_condition {
                    //info!("testing quantity");
                    let (farm_name, quantity) = by_names.front()?;
                    //info!("testing quantity: {}", quantity);
                    if *quantity == 1 {
                        let smart_farm = splash_dao_offchain::entities::onchain::smart_farm::SmartFarm {
                            farm_id: FarmId(spectrum_cardano_lib::AssetName::from(farm_name.clone())),
                            pool_id: conf.pool_id,
                        };
                        //info!("Going to return snapshot");
                        return Some(SmartFarm {
                            id: smart_farm.farm_id,
                            status: SmartFarmStatus::Free,
                            splash_qty,
                            ada_qty,
                        });
                    }
                }
            }
        };
        None
    }
}
