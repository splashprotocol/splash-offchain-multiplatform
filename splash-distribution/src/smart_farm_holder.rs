use async_trait::async_trait;
use cml_chain::transaction::TransactionOutput;
use cml_chain::Value;
use bloom_offchain::execution_engine::bundled::Bundled;
use bloom_offchain::execution_engine::liquidity_book::core::Unit;
use spectrum_cardano_lib::output::FinalizedTxOut;
use splash_dao_offchain::entities::onchain::smart_farm::SmartFarmSnapshot;
use crate::entities::smart_farm::{DistributorSmartFarmSnapshot, SmartFarm, SmartFarmStatus};

#[async_trait]
pub trait SmartFarmHolder {
    async fn get_free_farms_by_value(&self, value: Value) -> Vec<Bundled<DistributorSmartFarmSnapshot, TransactionOutput>>;

    async fn update_smart_farms_statuses(&self, farms: Vec<DistributorSmartFarmSnapshot>, status: SmartFarmStatus) -> ();
}