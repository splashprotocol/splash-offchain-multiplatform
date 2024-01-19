use bloom_offchain_cardano::creds::{ExecutorCred, RewardAddress};
use cardano_chain_sync::client::Point;
use cardano_explorer::data::ExplorerConfig;
use cml_core::Slot;
use spectrum_offchain_cardano::ref_scripts::ReferenceSources;
use std::time::Duration;

#[derive(serde::Deserialize)]
#[serde(bound = "'de: 'a")]
#[serde(rename_all = "camelCase")]
pub struct AppConfig<'a> {
    pub chain_sync: ChainSyncConfig<'a>,
    pub node: NodeConfig<'a>,
    pub tx_submission_buffer_size: usize,
    pub batcher_private_key: &'a str, //todo: store encrypted
    pub ref_scripts: ReferenceSources,
    pub explorer: ExplorerConfig<'a>,
    pub reward_address: RewardAddress,
    pub executor_cred: ExecutorCred,
    pub cardano_finalization_delay: Duration,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NodeConfig<'a> {
    pub path: &'a str,
    pub magic: u64,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChainSyncConfig<'a> {
    pub starting_point: Point,
    pub disable_rollbacks_until: Slot,
    pub db_path: &'a str,
}
