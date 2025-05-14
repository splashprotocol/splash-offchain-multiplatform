use crate::account::{AccountInPool, SuspendedPositionEvents};
use crate::feed::event::ExportAccountEvent;
use crate::onchain::event::{
    AccountEvent, FarmEvent, Harvest, MultipleAccountsHarvest, OnChainEvent, PoolEvent, PositionEvent,
};
use crate::position_db::accounts::Accounts;
use crate::position_db::pool_frames::PoolFrames;
use crate::position_db::{
    account_key, cred_index_key, export_feed, from_account_key, from_event_key, get_range_iterator, pool_key,
    sus_event_key, PositionDB, ACCOUNTS_CF, ACCOUNT_FEED_CF, ACTIVE_FARMS_CF, AGGREGATE_CF, CREDS_INDEX_CF,
    EVENTS_CF, MAX_BLOCK_NUM_KEY, POOL_LQ_FRAMES_INDEX_CF, SUS_EVENTS_CF,
};
use async_trait::async_trait;
use cml_chain::certs::Credential;
use log::info;
use rocksdb::{IteratorMode, ReadOptions};
use serde::{Deserialize, Serialize as SSerialize};
use spectrum_offchain_cardano::data::PoolId;
use std::collections::hash_map::Entry;
use std::collections::{HashMap, VecDeque};
use std::fmt::{Display, Formatter};
use cml_chain::PolicyId;
use cml_core::serialization::Serialize;
use tokio::task::spawn_blocking;
use spectrum_cardano_lib::{AssetName, Token};

#[async_trait]
pub trait MatureEvents {
    async fn try_process_mature_events(&self, confirmation_delay_blocks: u64) -> bool;
}

#[async_trait]
impl MatureEvents for PositionDB {
    async fn try_process_mature_events(&self, confirmation_delay_blocks: u64) -> bool {
        let db = self.db.clone();
        let result = spawn_blocking(move || {
            info!("[Mature events] Processing mature events");
            let aggregates_cf = db.cf_handle(AGGREGATE_CF).unwrap();
            let tx = db.transaction();
            if let Some(max_block_num) = tx
                .get_cf(aggregates_cf, MAX_BLOCK_NUM_KEY)
                .unwrap()
                .map(|raw| rmp_serde::from_slice::<u64>(&raw).unwrap())
            {
                info!("[Mature events] max_block_num: {}", max_block_num);
                let events_len = {
                    let events_cf = db.cf_handle(EVENTS_CF).unwrap();
                    let mut iter_events =
                        tx.iterator_cf_opt(events_cf, ReadOptions::default(), IteratorMode::Start);
                    let mut current_slot = None;
                    let mut events: Vec<OnChainEvent> = vec![];
                    let mut export_events = vec![];
                    while let Some(Ok((event_key, value))) = iter_events.next() {
                        let (block_num, _) = from_event_key(event_key.clone().to_vec()).unwrap();
                        if let Some(current_slot) = current_slot {
                            // if current_slot != block_num {
                            //     break;
                            // }
                        } else {
                            if max_block_num - block_num <= confirmation_delay_blocks {
                                return false;
                            }
                            current_slot = Some(block_num);
                        };
                        let event = rmp_serde::from_slice::<OnChainEvent>(&value).unwrap();
                        info!("[Mature events] Add event {}", event);
                        events.push(event);
                        tx.delete_cf(events_cf, event_key).unwrap();
                    }
                    // if events.is_empty() {
                    //     tx.commit().unwrap();
                    //     return false;
                    // }
                    let accounts_cf = db.cf_handle(ACCOUNTS_CF).unwrap();
                    let cred_index_cf = db.cf_handle(CREDS_INDEX_CF).unwrap();
                    if !(events.len() == 0) {
                        let mut formatted_events = String::new();
                        events.iter().clone().for_each(|event| {
                            formatted_events.push_str(format!(", {}", event).as_str());
                        });
                        info!("[Mature events] Events are (len {}) {:?}. Current_slot: {:?}", events.len(), formatted_events, current_slot);
                    }
                    let events_len = events.len();
                    let frames: HashMap<PoolId, PoolFrame> = aggregate_events(events);
                    for (pool_id, mut pool_frame) in frames {
                        info!("[Mature events] Processing pool {}. Pool frame is {}. Current_slot {:?}", pool_id, pool_frame, current_slot);
                        let mut lp_supply;

                        let pool_lq_frames_cf = db.cf_handle(POOL_LQ_FRAMES_INDEX_CF).unwrap();

                        // if there is no deposit, redeem events in frame we should restore
                        // previous frame lq_supply
                        if let Some(new_lq_supply) = pool_frame.lp_supply {
                            lp_supply = new_lq_supply
                        } else {
                            if let Ok(Some(raw_lq_value)) =
                                tx.get_cf(pool_lq_frames_cf, pool_key(pool_id))
                            {
                                lp_supply = rmp_serde::from_slice::<u64>(&raw_lq_value).unwrap();
                            } else {
                                info!(
                                    "No LQ supply found for pool {}. Skip processing events for this frame",
                                    pool_id
                                );
                                continue;
                            }
                        }

                        info!("[Mature events] Adding for pool {} new supply {}", pool_id, lp_supply);
                        info!("[Mature events] Key is: {}", hex::encode(&rmp_serde::to_vec(&pool_id).unwrap()));

                        let pool_key = pool_key(pool_id);

                        // update pool lq value
                        let put_result = tx.put_cf(
                            pool_lq_frames_cf,
                            pool_key.clone(),
                            &rmp_serde::to_vec(&lp_supply).unwrap(),
                        );

                        put_result
                            .map_err(|err| info!("[Mature events] Got error during updating pool lq: {:?}", err))
                            .map(|res| info!("[Mature events] Put result is {:?}", res))
                            .unwrap();

                        info!("[Mature events] Added to tx");

                        let current_slot = current_slot.unwrap();
                        let active_farms_cf = db.cf_handle(ACTIVE_FARMS_CF).unwrap();
                        info!("[Mature events] Processing farm events in frame");
                        for farm_event in pool_frame.farm_events {
                            match farm_event {
                                FarmEvent::FarmActivated(farm_activated) => {
                                    info!("[Mature events] Handle farm activated event for pool {} at slot {}", farm_activated.pool_id, farm_activated.slot);
                                    let value = rmp_serde::to_vec(&farm_activated.slot).unwrap();
                                    tx.put_cf(active_farms_cf, pool_key.clone(), value).unwrap();
                                }
                                FarmEvent::FarmDeactivated(farm_deactivated) => {
                                    info!("[Mature events] Handle farm deactivated event for pool {}", farm_deactivated.pool_id);
                                    tx.delete_cf(active_farms_cf, pool_key.clone()).unwrap();
                                }
                            }
                        }
                        let farm_activated_at = tx
                            .get_cf(active_farms_cf, pool_key.clone())
                            .unwrap()
                            .map(|raw| rmp_serde::from_slice::<u64>(&raw).unwrap());
                        info!("[Mature events] Farm for pool {} activated at {:?}", pool_id, farm_activated_at);
                        let mut iter_accounts = get_range_iterator(&db, accounts_cf, pool_key);
                        let mut accounts_for_update: HashMap<Credential, (AccountInPool, AccountFrame)> =
                            HashMap::new();
                        let suspended_events_cf = db.cf_handle(SUS_EVENTS_CF).unwrap();
                        while let Some(Ok((key, value))) = iter_accounts.next() {
                            let (_, account_cred) = from_account_key(key.to_vec()).unwrap();
                            info!("[Mature events] Processing account {}", hex::encode(account_cred.to_raw_bytes()));
                            let account = rmp_serde::from_slice::<AccountInPool>(&value).unwrap();
                            info!("[Mature events] Current account state {:?}", account);
                            let updated_account: AccountInPool = if let Some(farm_activated_at) = farm_activated_at {
                                account.activated(farm_activated_at)
                            } else {
                                account.deactivated()
                            };
                            let mut account_frame: AccountFrame = pool_frame
                                .account_frames
                                .remove(&account_cred)
                                .unwrap_or_else(|| AccountFrame::new());
                            let account_prefix = rmp_serde::to_vec(&account_cred.clone()).unwrap();
                            let mut iter_suspended_events =
                                get_range_iterator(&db, suspended_events_cf, account_prefix);
                            while let Some(Ok((key, value))) = iter_suspended_events.next() {
                                let suspended_events = rmp_serde::from_slice(&value).unwrap();
                                account_frame.suspended_position_events.push(suspended_events);
                                account_frame.suspended_position_events_keys.push(key.to_vec());
                            }
                            accounts_for_update.insert(account_cred, (updated_account, account_frame));
                        }
                        // Left events relate to yet non-existent accounts
                        for (new_account_key, account_frame) in pool_frame.account_frames {
                            info!("[Mature events] Processing non existent account {}]", hex::encode(new_account_key.to_cbor_bytes()));
                            accounts_for_update.insert(
                                new_account_key,
                                (
                                    AccountInPool::new(current_slot, farm_activated_at.is_some()),
                                    account_frame,
                                ),
                            );
                        }
                        for (account_cred, (account_state, mut account_frame)) in accounts_for_update {
                            let next_account_state =
                                if let Some(first_harvest) = account_frame.harvest_events.pop_front() {
                                    account_frame.suspended_position_events_keys.into_iter().for_each(
                                        |key| {
                                            tx.delete_cf(suspended_events_cf, key).unwrap();
                                        },
                                    );
                                    account_frame.harvest_events.into_iter().fold(
                                        account_state
                                            .harvest(account_frame.suspended_position_events, first_harvest),
                                        |st, ev| st.harvest(vec![], ev),
                                    )
                                } else {
                                    if account_state.should_unlock(current_slot) {
                                        account_state.unlock(account_frame.suspended_position_events)
                                    } else {
                                        account_state
                                    }
                                };
                            let account_key = account_key(pool_id, account_cred.clone());
                            let next_account_state = match next_account_state.try_adjust_position(
                                current_slot,
                                lp_supply,
                                account_frame.upstream_position_events,
                            ) {
                                Ok(next) => next,
                                Err((intact_account_state, suspended_events)) => {
                                    let suspended_events_key =
                                        sus_event_key(account_cred.clone(), current_slot);
                                    let suspended_events_value =
                                        rmp_serde::to_vec_named(&suspended_events).unwrap();
                                    tx.put_cf(
                                        suspended_events_cf,
                                        suspended_events_key,
                                        suspended_events_value,
                                    )
                                    .unwrap();
                                    intact_account_state
                                }
                            };
                            info!("[Mature events] New account state {:?}", next_account_state);
                            let updated_account_value = rmp_serde::to_vec_named(&next_account_state).unwrap();
                            info!("[Mature events] Persist account into accounts cf at key {}", hex::encode(account_key.clone()));
                            tx.put_cf(accounts_cf, account_key, updated_account_value)
                                .unwrap();
                            let cred_index = cred_index_key(&account_cred, pool_id);
                            info!("[Mature events] Persist account into cred_index cf at key {}", hex::encode(cred_index.clone()));
                            tx.put_cf(cred_index_cf, cred_index, vec![]).unwrap();
                            export_events.push(ExportAccountEvent {
                                account_cred,
                                pool_id,
                                update: next_account_state,
                            });
                        }
                    }
                    let account_feed_cf = db.cf_handle(ACCOUNT_FEED_CF).unwrap();
                    export_feed::batch_append(&tx, export_events, account_feed_cf);
                    events_len
                };
                // if !(events_len == 0) {
                //     info!("[Mature events] Before tx commit ---");
                //     tx.commit().unwrap();
                //     info!("[Mature events] after tx commit ---");
                //
                //     return true
                // }
            }
            info!("[Mature events] Before tx commit save");
            tx.commit().unwrap();
            info!("[Mature events] after tx commit save");
            return true
        })
        .await
        .unwrap();

        result
    }
}

fn aggregate_events(events: Vec<OnChainEvent>) -> HashMap<PoolId, PoolFrame> {
    let mut aggregated_events: HashMap<PoolId, PoolFrame> = HashMap::new();
    for event in events {
        let event_pid = event.pool_id();
        match aggregated_events.entry(event_pid) {
            Entry::Vacant(entry) => {
                let mut new_frame = PoolFrame::new();
                new_frame.apply_event(event);
                entry.insert(new_frame);
            }
            Entry::Occupied(mut entry) => {
                entry.get_mut().apply_event(event);
            }
        };
    }
    aggregated_events
}

#[derive(Debug)]
struct AccountFrame {
    harvest_events: VecDeque<Harvest>,
    suspended_position_events: Vec<SuspendedPositionEvents>,
    suspended_position_events_keys: Vec<Vec<u8>>,
    upstream_position_events: Vec<PositionEvent>,
}

impl AccountFrame {
    fn new() -> Self {
        Self {
            harvest_events: VecDeque::new(),
            suspended_position_events_keys: vec![],
            suspended_position_events: vec![],
            upstream_position_events: vec![],
        }
    }
    fn apply_event(&mut self, event: AccountEvent) -> Option<u64> {
        match event {
            AccountEvent::Position(p) => {
                let lp_supply = p.lp_supply();
                self.upstream_position_events.push(p);
                Some(lp_supply)
            }
            AccountEvent::Harvest(h) => {
                self.harvest_events.push_back(h);
                None
            }
        }
    }
}

#[derive(Debug)]
struct PoolFrame {
    farm_events: Vec<FarmEvent>,
    account_frames: HashMap<Credential, AccountFrame>,
    lp_supply: Option<u64>,
}

impl Display for PoolFrame {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        let mut farm_events = String::new();
        self.farm_events.iter().for_each(|event| {
           farm_events.push_str(format!(", {}", event).as_str());
        });
        write!(f, "PoolFrame (farm_events = {}, lp_supply = {:?})", farm_events, self.lp_supply)
    }
}

impl PoolFrame {
    fn new() -> Self {
        Self {
            farm_events: vec![],
            account_frames: Default::default(),
            lp_supply: None,
        }
    }
    fn apply_event(&mut self, event: OnChainEvent) {
        match event {
            OnChainEvent::Account(account_event) => {
                let maybe_lp_supply = match self.account_frames.entry(account_event.account()) {
                    Entry::Vacant(acc) => {
                        let mut new_frame = AccountFrame::new();
                        let lp_supply = new_frame.apply_event(account_event);
                        acc.insert(new_frame);
                        lp_supply
                    }
                    Entry::Occupied(mut acc) => acc.get_mut().apply_event(account_event),
                };
                if let Some(lp_supply) = maybe_lp_supply {
                    self.lp_supply.replace(lp_supply);
                }
            }
            OnChainEvent::FarmEvent(farm_event) => {
                self.farm_events.push(farm_event);
            }
            OnChainEvent::PoolEvent(pool_event) => match pool_event {
                PoolEvent::PoolCreated(pool_creation_event) => {
                    self.lp_supply.replace(pool_creation_event.supply_lq);
                }
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;
    use cml_chain::transaction::Transaction;
    use cml_core::serialization::{Deserialize, FromBytes, Serialize};
    use super::*;
    use crate::onchain::event::{Deposit, FarmActivated, FarmCreated, PollFactoryEvents, PoolCreated, StatelessOnChainEvent};
    use crate::position_db::event_log::EventLog;
    use crate::position_db::export_feed::ExportEventFeed;
    use cml_crypto::{Ed25519KeyHash, ScriptHash};
    use either::Right;
    use futures::FutureExt;
    use bloom_offchain_cardano::validation_rules::ValidationRules;
    use cardano_chain_sync::atomic_flow::BlockEvents;
    use cardano_explorer::AnyExplorer;
    use spectrum_offchain_cardano::persistent_index::IndexRocksDB;
    use splash_dao_offchain::deployment::ProtocolTokens;
    use splash_dao_offchain::entities::onchain::poll_factory::PollFactory;
    use splash_dao_offchain::entities::onchain::smart_farm::FarmId;
    use splash_dao_offchain::routines::Slot;
    use crate::config::AppConfig;
    use crate::context::Context;
    use crate::pipeline::resolve_gauges::resolve_gauges;
    use crate::pipeline::log_events::log_event;
    use crate::pipeline::read_events::read_events;
    use crate::ve_index::VoteEscrowDB;
    use spectrum_offchain_cardano::deployment::{
        DeployedValidators as DexValidators, ProtocolDeployment as DexDeployment,
    };
    use splash_dao_offchain::deployment::{
        DeployedValidators as DaoValidators, ProtocolDeployment as DaoDeployment,
    };
    use splash_testing::db_path::DBPath;
    use crate::position_db::cred_index_prefix;

    #[tokio::test]
    async fn some_test() {

        let cred_str = "8200581c8d4be10d934b60a22f267699ea3f7ebdade1f8e535d1bd0ef7ce18b6";

        let raw_cred = hex::decode(cred_str).unwrap();

        let cred_from_cbor = Credential::from_cbor_bytes(raw_cred.clone().as_ref());

        let cred_from_bytes = Credential::from_bytes(raw_cred.clone());

        let test_credential = cred_from_bytes.unwrap();

        let pid = PoolId::random();

        let key1 = cred_index_key(&test_credential, pid.clone());

        let key2 = cred_index_prefix(test_credential.clone());

        let a = 1;
        let b = 2;

        let test_1 = rmp_serde::to_vec(&a).unwrap();

        let test_2 = rmp_serde::to_vec(&(a, b)).unwrap();

        let key_length = test_credential.clone().to_cbor_bytes().len();
        let key_2_length = rmp_serde::to_vec(&test_credential.clone()).unwrap().len();

        let pid_cbor = hex::encode(rmp_serde::to_vec(&pid).unwrap());

        println!("credential length: {:?}", key_length);
        println!("credential length key_2_length: {:?}", key_2_length);

        println!("key1: {:?}", hex::encode(key1));
        println!("pid_cbor: {:?}", pid_cbor);
        println!("key2: {:?}", hex::encode(key2));

        println!("test_1: {:?}", hex::encode(test_1));
        println!("test_2: {:?}", hex::encode(test_2));

        let bytes: Vec<u8> = Vec::from(pid);
        let round_test = PoolId::try_from(&*bytes).unwrap();

        info!("{:?}", round_test);
        info!("{:?}", bytes);
        assert_eq!(1, 2)
    }

    #[tokio::test]
    async fn process_export_mature_events() {
        //let db_path = DBPath::new("/Users/aleksandr/IdeaProjects/spectrum-offchain-multiplatform/test");
        let db = PositionDB::new("/Users/aleksandr/IdeaProjects/spectrum-offchain-multiplatform/test");

        let pid = PoolId::random();
        let account = Credential::new_pub_key(Ed25519KeyHash::from([0u8; 28]));

        let r2 = (1_000u64, 1_000_000u64);
        let r3 = (1_000u64, 2_000_000u64);
        let r4 = (2_000u64, 8_000_000u64);

        // Generate a few OnChainEvents
        let event1 = OnChainEvent::FarmEvent(FarmEvent::FarmActivated(FarmActivated { pool_id: pid, slot: Slot(1) }));
        let event2 = OnChainEvent::Account(AccountEvent::Position(PositionEvent::Deposit(Deposit {
            pool_id: pid,
            account: account.clone(),
            lp_mint: r2.0,
            lp_supply: r2.1,
        })));
        let event3 = OnChainEvent::Account(AccountEvent::Position(PositionEvent::Deposit(Deposit {
            pool_id: pid,
            account: account.clone(),
            lp_mint: r3.0,
            lp_supply: r3.1,
        })));
        let event4 = OnChainEvent::Account(AccountEvent::Position(PositionEvent::Deposit(Deposit {
            pool_id: pid,
            account: account.clone(),
            lp_mint: r4.0,
            lp_supply: r4.1,
        })));

        db.batch_append(1, vec![event1, event2]).await;
        db.batch_append(10, vec![event3]).await;

        let ok = db.try_process_mature_events(5).await;

        assert!(ok);

        let Some((sn, export_event_1)) = db.next().await else {
            panic!("No event")
        };
        //db.delete(sn).await;
        info!("{:?}", export_event_1);
        assert_eq!(export_event_1.account_cred, account);
        assert_eq!(export_event_1.update.share, r2);

        db.batch_append(25, vec![event4]).await;
        let ok = db.try_process_mature_events(5).await;
        assert!(ok);

        let Some((sn, export_event_2)) = db.next().await else {
            panic!("No event")
        };
        //db.delete(sn).await;
        info!("{:?}", export_event_2);
    }

    #[tokio::test]
    async fn process_export_mature_events_with_pre_activated_farm() {

        let raw_config = std::fs::read_to_string("/Users/aleksandr/IdeaProjects/spectrum-offchain-multiplatform/splash-lp-indexer/resources/mainnet.config.json").expect("Cannot load configuration file");
        let config: AppConfig = serde_json::from_str(&raw_config).expect("Invalid configuration file");

        let raw_deployment =
            std::fs::read_to_string("/Users/aleksandr/IdeaProjects/spectrum-offchain-multiplatform/bloom-cardano-agent/resources/preprod.deployment.json").expect("Cannot load DEX deployment file");
        let dex_validators: DexValidators =
            serde_json::from_str(&raw_deployment).expect("Invalid deployment file");

        let raw_deployment =
            std::fs::read_to_string("/Users/aleksandr/IdeaProjects/spectrum-offchain-multiplatform/splash-lp-indexer/resources/dao_deployment.json").expect("Cannot load DAO deployment file");
        let dao_validators: DaoValidators =
            serde_json::from_str(&raw_deployment).expect("Invalid deployment file");

        let raw_tokens = std::fs::read_to_string("/Users/aleksandr/IdeaProjects/spectrum-offchain-multiplatform/splash-lp-indexer/resources/tokens.json").expect("Cannot load DAO assets file");
        let dao_tokens: ProtocolTokens = serde_json::from_str(&raw_tokens).expect("Invalid deployment file");

        let raw_validation_rules =
            std::fs::read_to_string("/Users/aleksandr/IdeaProjects/spectrum-offchain-multiplatform/bloom-cardano-agent/resources/validation-rules.json").expect("Cannot load bounds file");
        let validation_rules: ValidationRules =
            serde_json::from_str(&raw_validation_rules).expect("Invalid bounds file");

        log4rs::init_file("/Users/aleksandr/IdeaProjects/spectrum-offchain-multiplatform/splash-lp-indexer/resources/log4rs.yaml", Default::default()).unwrap();

        info!("Starting LP indexer ..");

        let explorer = AnyExplorer::new(&config.explorer, config.network_id)
            .await
            .expect("Explorer initialization failed");

        let dex_protocol_deployment = DexDeployment::unsafe_pull(dex_validators, &explorer).await;
        let dao_protocol_deployment = DaoDeployment::unsafe_pull(dao_validators, &explorer).await;


        let persistable_entites = HashSet::from([
            dex_protocol_deployment.balance_fn_pool_v1.hash,
            dex_protocol_deployment.balance_fn_pool_v2.hash,
            dex_protocol_deployment.const_fn_pool_v1.hash,
            dex_protocol_deployment.const_fn_pool_v2.hash,
            dex_protocol_deployment.const_fn_pool_fee_switch.hash,
            dex_protocol_deployment.const_fn_pool_fee_switch_v2.hash,
            dex_protocol_deployment.const_fn_pool_fee_switch_bidir_fee.hash,
            dex_protocol_deployment.royalty_pool.hash,
            dex_protocol_deployment.stable_fn_pool_t2t.hash,
            dao_protocol_deployment.smart_farm.hash,
            dao_protocol_deployment.farm_factory.hash,
            dao_protocol_deployment.wp_factory.hash,
            dao_protocol_deployment.ve_factory.hash,
        ]);
        let cx = Context {
            dex_deployment: dex_protocol_deployment,
            dao_deployment: dao_protocol_deployment,
            dao_tokens,
            pool_validation: validation_rules.pool,
            harvest_limits: config.harvest_limits,
        };

        //let db_path = DBPath::new("/Users/aleksandr/IdeaProjects/spectrum-offchain-multiplatform/test");
        let db_path = DBPath::new("process_export_mature_events_with_pre_activated_farm");
        let db = PositionDB::new(&db_path);
        let gauges_db_path = DBPath::new("gauges_index");
        let gauges_db = VoteEscrowDB::new(&gauges_db_path);

        let utxo_db_path = DBPath::new("utxo_index");
        let utxo_index = IndexRocksDB::new(&utxo_db_path);

        let pid = PoolId::random();
        let fid = FarmId::random();
        let account = Credential::new_pub_key(Ed25519KeyHash::from([0u8; 28]));

        let r2 = (1_000u64, 1_000_000u64);
        let r3 = (1_000u64, 2_000_000u64);
        let r4 = (2_000u64, 8_000_000u64);

        let pool_created_tx_raw = "84a8008282582001840867fd3ba17a5a57b14c88cd1b8e8c990015f3a62ccf94e6d4ef12953d9d018258200e67252460f44f0336418edd65e3d0d195151d0733963c97ce2033711703d026010183a3005839309dee0659686c3ab807895c929e3284c11222affd710b09be690f924db2f6abf60ccde92eae1a2f4fdf65f2eaf6208d872c6f0e597cc10b0701821a0bebc200a3581c92b0e610665eb5258c75d6747e9acf41e383ffaccd1cc82aeefee41ca14364646418c8581cb5e8836450525a945852ad7e1648dc8ce26bfc4d495496bfdca834e4a14b6464645f4144415f4e465401581ceb456f370345f8799909558e99b66649b1b821fbb729c485b73104dba14a6464645f4144415f4c511b7ffffffffffcf2bf028201d81858d3d8798bd87982581cb5e8836450525a945852ad7e1648dc8ce26bfc4d495496bfdca834e44b6464645f4144415f4e4654d879824040d87982581c92b0e610665eb5258c75d6747e9acf41e383ffaccd1cc82aeefee41c43646464d87982581ceb456f370345f8799909558e99b66649b1b821fbb729c485b73104db4a6464645f4144415f4c511a00018592181e000081d87981d87a81581c9d9318089a77c75b918ac352c64f6cfb16e36002d9ed684421f1f68400581c75c4570eb625ae881b32a34c52b159f6f3f3f2c7aaabf5bac4688133825839008d4be10d934b60a22f267699ea3f7ebdade1f8e535d1bd0ef7ce18b681d87f73eca0cc06a5a85cc9b418fb735410050e5dedadcc1d20a51d821a00d46c14b84a581c03b896e0915daea70a129ca5e9d087b9e63405a55aa9d3d21f7e3c33a1426c711a0010f446581c06fdee504f461905e391123692c22db9fb931d24e562469d0831df2da1437474741a2a832171581c08f5b7433544c5f5d8921cb3db337967ec916e969c8a8db886a61221a1426c711a0bebc200581c0960be315376c344e65f01bb0a9b6eb9fecb654a71a1f5e78068be55a1426c711a004c4b3f581c0a82e80ee14234a3add504880dfad89c261ca44520984134562ec945a1426c711a004c4b3f581c0a99225abbcedc059c822a3437659b4020bcbe4d701e6d07f300a925a153536e656b2e66756e2074657374202d204e465401581c0df79145b95580c14ef4baf8d022d7f0cbb08f3bed43bf97a2ddd8cba1426c711a04d3f629581c0e9b623fce08f4485e24fec7eef9a40652b783a732dbf9b983580831a1426c711a00a98ac6581c1036770796e0e6552ae8d5595722e91038ac7d9ae3e9e397d5e70069a1426c711b7fffffffffffffff581c15c69680b39c9239b2245aa2576f18209f2445256fbfd30145e316b3a1426c711a00ed5be2581c187f7eb449051613d913464bd1d883a689fe3b71b2d1458237f2355da1426c711a00cb7354581c18849736304810b27c059c23000911b1ea76ade53744cd94e761ce5ea1426c711a0bebc200581c27dfc16384cb64ca3fd0340f5bfdfcf4f77591d59bfca01945b51099a1426c711abb7c8560581c33858d11ae77911cb1e064967686b984afaf10f193fee27799cb110ea1426c711a0bebc200581c346a4d71533e28725bc58b8ef9df604d6ba347582ed168fc76717d3fa152536e656b2e66756e20747474202d204e465401581c3bd5e121402c9091467c470fe2b85b1b504015bcd56257d95c56071ca1426c711a0bebc200581c41fe9380e7070cb77a661629fdca75b6b3cb62b3732c78a0136cb382a1426c711a05f5e0ff581c433509753796c8ca98f2624bba59d10079b9b0acdc384958c04225e3a152536e656b2e66756e20717765202d204e465401581c44b266c58799a4d0c2addec60e17c83bb0c691227e5f29fb81014cb7a1436e667401581c49770dbf18e37a6051d19502995c76004330a13558b82b5c945cb9d2a1426c711a0bebc200581c49a500347508220d9fa60e237a76f05d3b2137f60a6f6b3d0b1477aba1426c711a00cb7354581c4b3459fd18a1dbabe207cd19c9951a9fac9f5c0f9c384e3d97efba26a24574657374431b299b07bab97d7db24574657374441b29a2241a7d2082d4581c4cdfa778ac5b3ee3ed68325b9e6a9aa887a4c4c1379479cc7f6105a5a1426c711a0bebc200581c4dfd22c70d00f9dedb5bfea730211f2d12ffcfffa0f3db754bde1181a1426c711a00a98ac6581c4ebbd7d6e2bb8cd68400309623ad1e2a8bb1be9f96842bebcc48ca58a1426c711a00cb7354581c4f281a6cc4680bf739309f95cd7a9f0cb52d659e760f78a295701f95a1426c711a0bebc200581c56a08452059b6a98f2d10186a410aed5a2a9132227a9896ba9413aa3a1426c711a0bebc200581c5a06ac7ffe4539e4ca0fb71fd1ae0725eadd39f1a7a6ade80e04e1f0a1426c711a0064aba3581c5a6e35e842d82500f37cab442501408fd9df10f9ba65596a079d2318a1426c711af6aee41e581c5ac2b7b8f41ff6832eb57d9cacb434a3b11650e60455faad492520d3a14c6e66746e66746e66746e667401581c5dacaa165a61f9757a3023b834145aa4fafec24bfaca4b90c11ea5d5a1426c711a00e747ad581c5f3bd8784c97db71bcd3ddf6f69243c98a448816b89aeaa77177ede1a1426c711a0bebc200581c6345cb1e61f07faee94caa4a921c24b0e0027cc7433a92a6ec268228a1426c711a05f5e0ff581c634cca6e663ee3a9ecf24d336b70615722a4881a239bbc76105df95ba1426c711a0bebc200581c6d71571364f43f8a88463570ed2d4edba2314f596e65ccaab5d5fd7aa14c74657374435f4144415f4c511b000000075cdd4331581c6de15b4f945166e44fe7984ac89918e899c348a2357732122d4ecb6fa1467177657274791a0740ad2a581c70763137771a095959ce09e676c4303b5c3e878741b74ce93182a4eaa1426c711a0bebc200581c7230396dd93dc16f6e19279a418bf114f301361612351694847ca9d4a1426c711a00145854581c764ab49b2ac607edba6a61b1c2745fb362e17bf0245fec0989c84b32a1426c711a00a98ac6581c775cd514f10ee48b8eb93e036906b560fc18e65085592d64dd15b5f7a1426c711a0bebc200581c7adf9da5cf304976ded41896f9453ce5d706fecda4d28362d105c3b0a1426c711a0bebc200581c7de57deab376ad1aea0370e9713a1ff420bbc5501a5b26ecfa2458aaa1436e667401581c8223c45aa673fcfce11ef6d20931a857e2feea5dcb05bdc90f87a5a0a1426c711a0bebc200581c82d6fd14f2030454326f9bcda4fb95fa8fa7b3d98e571ca7ad1adcbaa1436e667401581c8c9f10bf2bc9cd491054ae418fbe51d4d94c5dba8f7dda74d46da0c4a1426c711a05f5e0ff581c8cb7b7534fb066f7ddc6539657da8b2cc943c8df98482fc77fa59cb3a1426c711a0bebc200581c8dcb9153bddbac9558d41c8e3b103f858ecfda1381b7b15cbd9ab25ea1426c711a9e044eda581c8fa6738a791b1cd5f85a5b741dc0bd8a18c223bcda6c89dd52ceaa04a1426c711a0bebc200581c907f49f40f27db8e35088a45223b0f3e492bf2c4ae80d56445c65f6da1426c711a004c4b3f581c90b07ec79b70048da8c67f88c0d71d81c0cf954375d15de521e97116a1426c711a00e747ad581c92b0e610665eb5258c75d6747e9acf41e383ffaccd1cc82aeefee41ca1436464641a077f1f19581c92df5cf0d96d8deacebfae0d68e930c2b6190cad649d181c3f6a7280a1426c711a0bebc200581c98d877f665a274a189a9dd8b45c2886532f0536f144b14a3ac402591a1426c711a0087a238581c9cdd1a4526e2421d2c9428f9c82afa399e9f275c8e2cc77062051551a1426c711a02625a00581ca0c105de55cfb78fc358eb931bfe998c033bbfe83b5e5841ef2f2faba1426c711a0bebc200581ca48393deaa97d6f71329988497081b5456d32362a1a12f3c92a366aca1426c711a77912049581ca4e4afc366efa742b01f7155f1da6480388b6600ceb21f76e4b1f9e9a155536e656b2e66756e20717765727479202d204e465401581caa39978058d7e02e28b7969e91f4360e48c549e063b40b066eab2ed7a1426c711a0bebc200581cac7d3b884e03322cb8984a9911630ddc3740e4a2de1bdee71047931ba1426c711a0010f446581cac88439fe8ec09c04a215d0d10a8e358186eba0aa99e93c3168f1eeaa1426c711a0bebc200581caf38202536688229dc2f641690d25830e8e361245cdec5467521d7a7a1426c711a05f5e0ff581caf5e3707b0a8c363b94c8f356259670f2cd54149c46344152c242c39a1426c711a01312d00581cb69484d6427c060738a82b1d7c5ab7ab0ddb77dbbfc1e04a4e2aa130a1426c711a0bebc200581cc5dc1c9ac1482035bd6caa8290ea9f41567aaa193cd8c5a336ddc57da1426c711a004c4b40581cc7b8094f17a44732882dd34912c09472a8d5fd8924539c8c02941d45a1426c711a0bebc200581cdf2d3eaa84280e77d00400feeb9e1d10f59f02c5e7c611b61e11edefa1426c711a0bebc200581cdf6d5ca3dc94294eaab387c1a47905b1acd72b9b0b112b57ef642bb8a152536e656b2e66756e20646464202d204e465401581ce06e64e5b0736649cf898a88618ccdf35540ccad8b152012948d854ea1426c711a0bebc200581ce19cb83c3fee2e5edd4eb71453d495758f27ce7502add42ad615fb3ba1466e66746e667401581ce28fbd007c74be6ab2db7493f58c8039ca4a0a81c79442db7979f108a1426c711a00e747ad581ceb456f370345f8799909558e99b66649b1b821fbb729c485b73104dba14a6464645f4144415f4c511a00030958581cee54cf3c92081bd85332c5ac9808507941ebf80de6dc8fe6e5d0bcd4a1426c711a07829b7b581cf36a15caef79f98d13ee26ea90c75a2a8f838f4eae88ad1e263a1ec7a1437177651a2dcbe09a581cf7adc3aa4fd018962ac3dd5fcef565b99388e517538eb02383577198a1426c711a0bebc200825839008d4be10d934b60a22f267699ea3f7ebdade1f8e535d1bd0ef7ce18b681d87f73eca0cc06a5a85cc9b418fb735410050e5dedadcc1d20a51d1b000000011ec82e9e021a0018bb9709a2581cb5e8836450525a945852ad7e1648dc8ce26bfc4d495496bfdca834e4a14b6464645f4144415f4e465401581ceb456f370345f8799909558e99b66649b1b821fbb729c485b73104dba14a6464645f4144415f4c511b7ffffffffffffc170b582042093730df0da5dd37a5bc033deeeb54fafc7e335479504c3348bec9be225a0e0d81825820e3efea3299dfb32f6133c1841e060df66cb2c04c0f7364d5c9c50316d532d8d10110825839008d4be10d934b60a22f267699ea3f7ebdade1f8e535d1bd0ef7ce18b681d87f73eca0cc06a5a85cc9b418fb735410050e5dedadcc1d20a51d1a001fa75c111a0044aa20a30081825820b2f97417886f87a990b7f2a6c78c8210d6bcf8c93498d6a5c54f7026a9948d75584079f18e6cb5681f1f10224a657c96f8eb650ce3a9e5d7d7e012b3fbf393a67d0199efa9c9f6dcf30e97c02922ec1d781f6d4319b8b42be6475981cdcfdbdd150f058284010000821a0004944f1a092aed0984010101821a009896801b0000000218711a00068259017959017601000032323232323232323232322253330083232325332233300d00200114a0666010444a666018002294054ccc038c008c04400452889980180118080009199119baf374e60240046e9cc048004c03cc04000530012bd8799fd8799f582001840867fd3ba17a5a57b14c88cd1b8e8c990015f3a62ccf94e6d4ef12953d9dff01ff000011323370e64646644666601600490001199980600124000eb4dd58008029bae3011001375c602260200026022002664466e9520003300d37520046601a6ea40052f5c0646464a66601e66e1d20000021375c60240022c6026004601c0026ea8c8c040c03c004c0400152210b6464645f4144415f4e46540048008dd598071918071807180700098068011bac300d001300d001300b300c0011498588c008dd480091111980291299980400088028a99980519baf300b300d00100613004300f300d00113002300c0010012323002233002002001230022330020020015573eae815cd2ab9d5744ae848c008dd5000aab9e0159018159017e01000032323232323232323232322253330083232325332233300d00200114a0666010444a666018002294054ccc038c008c04400452889980180118080009191919baf374e60240046e9cc0480053012bd8799fd8799f582001840867fd3ba17a5a57b14c88cd1b8e8c990015f3a62ccf94e6d4ef12953d9dff01ff00300f30100010011323370e64646644666601600490001199980600124000eb4dd58008029bae3011001375c602260200026022002664466e9520003300d37520046601a6ea40052f5c0646464a66601e66e1d20000021375c60240022c6026004601c0026ea8c8c040c03c004c04001522010a6464645f4144415f4c5100482bbc3fffffffffffffc04dd598071918071807180700098068011bac300d001300d001300b300c0011498588c008dd480091111980291299980400088028a99980519baf300b300d00100613004300f300d00113002300c0010012323002233002002001230022330020020015573eae815cd2ab9d5744ae848c008dd5000aab9e01f5f6";

        let pool_created_tx = Transaction::from_cbor_bytes(
            hex::decode(pool_created_tx_raw).unwrap().as_ref()
        ).unwrap();

        let pool_created_tx_block = BlockEvents::RollForward {
            events: vec![Right(pool_created_tx)],
            block_num: 50,
            block_slot: 100,
        };

        read_events(
            pool_created_tx_block,
            &cx,
            &utxo_index,
            &persistable_entites
        ).then(|events| resolve_gauges(
            events,
            &gauges_db,
            &db
        )).then(|events| log_event(events, &db)).await;

        let empty_events : Vec<OnChainEvent> = vec![];

        db.batch_append(110, empty_events).await;

        let ok = db.try_process_mature_events(5).await;
        assert!(ok);

        let farm_activated_tx_raw = "84a500d90102818258201a1b53aef701c194ada363bf35d063ec56be430ee48bdce9c097f01246f295d0070187a300581d70b2ae8d94c41de484cbf5c30f5a9ee74a9057dfb02f1dd4bf62b07f3301821a00130196a2581c63a4f24efff23238e1ac5158a2b4ce1a7aaf6cebb8c6915f75d972b6a141a401581c551a2b469e168128f53e81c61bbd9a0d3470ca46924830d542a16584a14653504c4153481b00001d1a94a20000028201d8184100a300581d70aad3f6f08a26048b07c5d53f9f07411962a8841b350c314ecbf5e6ce01821a001e8480a1581cbe1ae5c0c60d1dca30113e4cb2f94ee8eddb6ee6e3d6316b3c019f94a141a401028201d8185824d8798220581e581c2f50134295af8c9197f30e3fd9df164d190bb001d0f5d6c64f8ef48ba300581d7014e3dc8b0b152e31f4b0ece7fcb8d538bc0b017a5b75584245ac78d701821a001092a8a1581c43ecc63ca36d76042fdac669a140f0495cb7d82eec470f58a4d9a0fda141a401028201d8184bd879822083410041014102a300581d70db6050f98887359ed3fae914dbac615bceb47755b4181275030611ad01821a0015f734a2581c2fc4af50dd2d339345d7fc13f9ac5b8dd47d9fe6ae8f40459a912f34a141a401581c27b20af131ddff716b7d32dd29192a65a81a4645f6205bd0e7d2d490a141f41b009fdf42f6e48000028201d8185832d87982a1d87982581c551a2b469e168128f53e81c61bbd9a0d3470ca46924830d542a165844653504c415348d879820101a0a300581d70f327100fbd4320c1cebb535d09aa6d486cb7a5bd327b4bc3baedb0f8011a000dade0028201d81843d87980a300581d70bce96f6c202a99e58e4d3079b93df0a2b35fef1982570dae0498d57301821a001430a2a1581c2f50134295af8c9197f30e3fd9df164d190bb001d0f5d6c64f8ef48ba141a401028201d8185841d8798282581c3ebe44c8660ee99edd6f462455109117f0c8a96d06825212fea79be8581c52202b0a9aa9797c6d22b29d0bcd2b2782aaca47db097aa99428f45c8082583900e03edb696ad29c9b1cac96767147d8db4d2d28b9f4c568867054558784b33df17e33825174c50033f806b9d798e2961611b11a11e970a930821b00000002033e56b1b2581c82b117fbacd31918d5b67dbfb2ab42c28aaa098748527b7c530acbcfa141a401581cc4d19ee0ac465ffc32f9264398a3a825d861895752e4f9bad74083c7a141a401581cb35d3818ff3f2eb96c5e29458c0e464e85ad722277ccc8dcc146f442a141a401581c3aeeff5de2e96a735fe9fef1b08aca28b68fe73d8513a2b61fb1a2eda14653504c4153481b7fffe2e56b5af2bf581c551a2b469e168128f53e81c61bbd9a0d3470ca46924830d542a16584a14653504c4153481b7fffc5cad5e96dbf581c4364178b675b6ca944efa325c8d35ffb453b0a9e2ec1352bc5d33a85a141a401581ce081e494d2c8ff7b4afd48e86c3cf041dd78e18571d945fcf9279b37a14653504c4153481b7fffe2e56b596c1f581c3851e79be4a1f586e787a435747dfd74a76d91833be4d264eb4b16d3a141a401581cc4161784c5ea2024c70c9fb7b986e8674fb6008dcc5238d7e7ec4daea14653504c4153481b7fffe2e56b5c795f581c155688e6137018f00580fb88633735d8cbabb3a3c4c87025b5a0d8eca141a401581caed295bca76861d9923ca131633bf3c82209d8cc356d7441d5ddb4aaa141a401581cb936bd21aaedbc6a30ac319fce3fa9daa3b869e7b7e32f5c45e3b8aba141a401581c5217b7b87ce06d94ac4cd2fa05764240cbb18a647f598c3d0203f380a14e53504c4153482f414441204c512a1b00001d1a94a20000581c0d8ec5d3fbfec27ceec28ffd398dbf1a7ff28584d4f3e8d1be23cebda141a401581c8451ce79aafb9e5deca4e400d32eae1882d59e2a5669d8c7d923d34aa14e53504c4153482f414441204c512a1b00001d1a94a20000581ceca291b92576d8f1ae6eebac087ab92a1bb5eccf3fc259234db2a39ea14e53504c4153482f414441204c512a1b00001d1a94a20000581c46e6eee4f380a219cfcafede79968353eb94f0b87f87ff1fe538ff8aa14e53504c4153482f414441204c512a1b00001d1a94a20000581c2b43c144b49e4422eeb02d2c785740a1121eace7775e246396335d48a141a401021a0007b5bd0dd9010281825820779760239b94b532cbec67d3ec0f424d6058b007861bb2ba3f0dfb6d8ba70c520012d901028682582072f8439fc0bb72f888293515e008f9c38a72f3d240f86c6ac68376f284331f9b0082582072f8439fc0bb72f888293515e008f9c38a72f3d240f86c6ac68376f284331f9b0282582072f8439fc0bb72f888293515e008f9c38a72f3d240f86c6ac68376f284331f9b0382582072f8439fc0bb72f888293515e008f9c38a72f3d240f86c6ac68376f284331f9b0482582053b92c52e460381bfe640299d37735c3c905555359e3030cb40d9fd627824bd80082582053b92c52e460381bfe640299d37735c3c905555359e3030cb40d9fd627824bd801a100d90102818258204492c05037d24d626d50b2880aee1d83fcbe0d52cf425820a05bbf21812166a35840cade6b8ccd2473b7ffc0a27160376f5c1292ac6d01d46e5806652eb010fb9c9780dbacd560e9b7626f4eb9ab8488c72205d6b73bbcc79cf45fe0e4b29629400cf5f6";

        let farm_activated_tx = Transaction::from_cbor_bytes(
            hex::decode(farm_activated_tx_raw).unwrap().as_ref()
        ).unwrap();

        let farm_activated_tx_block = BlockEvents::RollForward {
            events: vec![Right(farm_activated_tx)],
            block_num: 80,
            block_slot: 130,
        };

        // let farm_activated_event = StatelessOnChainEvent::PollFactory(
        //     PollFactoryEvents::NewFactory(PollFactory {
        //         last_poll_epoch: None,
        //         active_farms: vec![
        //             fid.clone(),
        //         ],
        //         stable_id: ScriptHash::from([0; 28]),
        //     })
        // );
        //
        // let farm_activated_block = BlockEvents::RollForward {
        //     events: vec![farm_activated_event],
        //     block_num: 80,
        //     block_slot: 130,
        // };

        read_events(
            farm_activated_tx_block,
            &cx,
            &utxo_index,
            &persistable_entites
        ).then(|events| resolve_gauges(
            events,
            &gauges_db,
            &db
        )).then(|events| log_event(events, &db)).await;

        let empty_events : Vec<OnChainEvent> = vec![];

        db.batch_append(140, empty_events.clone()).await;

        let ok = db.try_process_mature_events(5).await;
        assert!(!ok);

        let farm_created_tx_raw = "84a700d9010282825820f34342bc95f776927ae9325998e061f1988887324b6d26d779e5a7b54f4b323900825820f34342bc95f776927ae9325998e061f1988887324b6d26d779e5a7b54f4b3239020183a300581d70aad3f6f08a26048b07c5d53f9f07411962a8841b350c314ecbf5e6ce01821a001e8480a1581cbe1ae5c0c60d1dca30113e4cb2f94ee8eddb6ee6e3d6316b3c019f94a141a401028201d8185824d8798202581e581c2f50134295af8c9197f30e3fd9df164d190bb001d0f5d6c64f8ef48ba300581d70278dce1b47da9cb4642bafb2e0e3379c3b4ae67525e61d1d129f3e0901821a0018fdbca1581c278dce1b47da9cb4642bafb2e0e3379c3b4ae67525e61d1d129f3e09a1410201028201d8185850d8799f581c2f50134295af8c9197f30e3fd9df164d190bb001d0f5d6c64f8ef48bd8799f581cb5e8836450525a945852ad7e1648dc8ce26bfc4d495496bfdca834e44b6464645f4144415f4e4654ffff82583900e03edb696ad29c9b1cac96767147d8db4d2d28b9f4c568867054558784b33df17e33825174c50033f806b9d798e2961611b11a11e970a930821b0000000202d3a7b4b2581caed295bca76861d9923ca131633bf3c82209d8cc356d7441d5ddb4aaa141a401581c2b43c144b49e4422eeb02d2c785740a1121eace7775e246396335d48a141a401581c3851e79be4a1f586e787a435747dfd74a76d91833be4d264eb4b16d3a141a401581ce081e494d2c8ff7b4afd48e86c3cf041dd78e18571d945fcf9279b37a14653504c4153481b7fffe2e56b596c1f581cc4d19ee0ac465ffc32f9264398a3a825d861895752e4f9bad74083c7a141a401581cb936bd21aaedbc6a30ac319fce3fa9daa3b869e7b7e32f5c45e3b8aba141a401581c3aeeff5de2e96a735fe9fef1b08aca28b68fe73d8513a2b61fb1a2eda14653504c4153481b7fffe2e56b5af2bf581c551a2b469e168128f53e81c61bbd9a0d3470ca46924830d542a16584a14653504c4153481b7fffc5cad5e96dbf581c5217b7b87ce06d94ac4cd2fa05764240cbb18a647f598c3d0203f380a14e53504c4153482f414441204c512a1b00001d1a94a20000581c0d8ec5d3fbfec27ceec28ffd398dbf1a7ff28584d4f3e8d1be23cebda141a401581cb35d3818ff3f2eb96c5e29458c0e464e85ad722277ccc8dcc146f442a141a401581cc4161784c5ea2024c70c9fb7b986e8674fb6008dcc5238d7e7ec4daea14653504c4153481b7fffe2e56b5c795f581c82b117fbacd31918d5b67dbfb2ab42c28aaa098748527b7c530acbcfa141a401581c8451ce79aafb9e5deca4e400d32eae1882d59e2a5669d8c7d923d34aa14e53504c4153482f414441204c512a1b00001d1a94a20000581c155688e6137018f00580fb88633735d8cbabb3a3c4c87025b5a0d8eca141a401581c4364178b675b6ca944efa325c8d35ffb453b0a9e2ec1352bc5d33a85a141a401581c46e6eee4f380a219cfcafede79968353eb94f0b87f87ff1fe538ff8aa14e53504c4153482f414441204c512a1b00001d1a94a20000581ceca291b92576d8f1ae6eebac087ab92a1bb5eccf3fc259234db2a39ea14e53504c4153482f414441204c512a1b00001d1a94a20000021a000a869709a1581c278dce1b47da9cb4642bafb2e0e3379c3b4ae67525e61d1d129f3e09a14102010b5820e1a5b457a57da8bc6c139aeb9c833df9600718403e4bd6178708f195e47c23180dd9010281825820779760239b94b532cbec67d3ec0f424d6058b007861bb2ba3f0dfb6d8ba70c520012d901028282582072f8439fc0bb72f888293515e008f9c38a72f3d240f86c6ac68376f284331f9b028258201a1b53aef701c194ada363bf35d063ec56be430ee48bdce9c097f01246f295d002a200d90102818258204492c05037d24d626d50b2880aee1d83fcbe0d52cf425820a05bbf21812166a35840c40c4debd5c711bd44add7f147c6f0d768f16b37eb06c3d21a92f69bb06a3bc40b40561b704da840ab57b7a5119ddfdd3c7436ef3770e41e438615f57fefb2040582840000d87980821a0007a1201a0bebc200840100d8798100821a0007a1201a0bebc200f5f6";

        let farm_created_tx = Transaction::from_cbor_bytes(
            hex::decode(farm_created_tx_raw).unwrap().as_ref()
        ).unwrap();

        let farm_created_tx_block = BlockEvents::RollForward {
            events: vec![Right(farm_created_tx)],
            block_num: 110,
            block_slot: 170,
        };

        //
        // let farm_created_event = StatelessOnChainEvent::FarmCreated(
        //     FarmCreated {
        //         farm_id: fid,
        //         pool_id: pid,
        //     }
        // );
        //
        // let farm_created_block = BlockEvents::RollForward {
        //     events: vec![farm_created_event],
        //     block_num: 110,
        //     block_slot: 140,
        // };

        read_events(
            farm_created_tx_block,
            &cx,
            &utxo_index,
            &persistable_entites
        ).then(|events| resolve_gauges(
            events,
            &gauges_db,
            &db
        )).then(|events| log_event(events, &db)).await;

        let empty_events : Vec<OnChainEvent> = vec![];

        db.batch_append(180, empty_events.clone()).await;

        let ok = db.try_process_mature_events(5).await;
        assert!(ok);

        let user_deposit_tx_raw = "84a600d90102828258205a37e7dff6e7bc49d3001553b0144dcb9a13f3b45f0a1819335b06b16cfd5aaa00825820cbcf6d594fa89cc6f487854775cfc18cd577f60a308f76d4ad3813d04276b900000183a3005839309dee0659686c3ab807895c929e3284c11222affd710b09be690f924db2f6abf60ccde92eae1a2f4fdf65f2eaf6208d872c6f0e597cc10b0701821a11e1a300a3581c92b0e610665eb5258c75d6747e9acf41e383ffaccd1cc82aeefee41ca14364646419012c581ceb456f370345f8799909558e99b66649b1b821fbb729c485b73104dba14a6464645f4144415f4c511b7ffffffffffb6c1f581cb5e8836450525a945852ad7e1648dc8ce26bfc4d495496bfdca834e4a14b6464645f4144415f4e465401028201d81858d3d8798bd87982581cb5e8836450525a945852ad7e1648dc8ce26bfc4d495496bfdca834e44b6464645f4144415f4e4654d879824040d87982581c92b0e610665eb5258c75d6747e9acf41e383ffaccd1cc82aeefee41c43646464d87982581ceb456f370345f8799909558e99b66649b1b821fbb729c485b73104db4a6464645f4144415f4c511a00018592181e000081d87981d87a81581c9d9318089a77c75b918ac352c64f6cfb16e36002d9ed684421f1f68400581c75c4570eb625ae881b32a34c52b159f6f3f3f2c7aaabf5bac4688133a2005839008d4be10d934b60a22f267699ea3f7ebdade1f8e535d1bd0ef7ce18b681d87f73eca0cc06a5a85cc9b418fb735410050e5dedadcc1d20a51d01821a0016e360a1581ceb456f370345f8799909558e99b66649b1b821fbb729c485b73104dba14a6464645f4144415f4c511a000186a0825839008db74620d771d9d35330347462c1fee2b706ef818bffc33bdc6f19883cccadcb9ccc0691b2856a08bb46c7ab71b65b49444452bb745812f11a000f58d3021a00078a8d0b582014150bafb2b8be1c869ee0eb43012a287f6dc3dc138f2b20a7ffdd72e50e6f5a0dd901028182582082414200b2e145f46b0e0ff0964b848da1f4630a71d90b6b580d8ef49ef560d30012d9010282825820a85dbebcf7a9c27a2548191e6516b12140ec744c142248355ddeef8652ab071f008258206eebdea4d351d198bc93452afba5b9cb4420617e6c94339f5b85f827c0f7447900a200d901028182582026bb5d1c8d31072cb8e9edfca1fff5d6b0f7b8007cf5115ad736a25b7cde65215840a33e95060efa38d8c47a0f30c4272f1ee5eabf7fb643d85c2bfedd197d8b0a0e54ed20374e6394903203c4ac3e6710dd0e02368b39d919da569484570c5c12030582840000d8798401000100821a0007a1201a0bebc200840001d879820001821a0009eb101a0f7f4900f5f6";

        let user_deposit_tx = Transaction::from_cbor_bytes(
            hex::decode(user_deposit_tx_raw).unwrap().as_ref()
        ).unwrap();

        let user_deposit_tx_block = BlockEvents::RollForward {
            events: vec![Right(user_deposit_tx)],
            block_num: 140,
            block_slot: 200,
        };

        read_events(
            user_deposit_tx_block,
            &cx,
            &utxo_index,
            &persistable_entites
        ).then(|events| resolve_gauges(
            events,
            &gauges_db,
            &db
        )).then(|events| log_event(events, &db)).await;

        db.batch_append(210, empty_events).await;

        let ok = db.try_process_mature_events(5).await;
        assert!(ok);

        // let user_deposit_event = StatelessOnChainEvent::Position(
        //     PositionEvent::Deposit(Deposit {
        //         pool_id: pid,
        //         account: account.clone(),
        //         lp_mint: r2.0,
        //         lp_supply: r2.1,
        //     })
        // );
        //
        // let user_deposit_block = BlockEvents::RollForward {
        //     events: vec![user_deposit_event],
        //     block_num: 140,
        //     block_slot: 170,
        // };

        // resolve_gauges(
        //     user_deposit_block,
        //     &gauges_db,
        //     &db
        // ).then(|events| log_event(events, &db)).await;

        // db.batch_append(50, vec![pool_created_event]).await;
        // db.batch_append(70, vec![farm_created_event]).await;

        // let ok = db.try_process_mature_events(5).await;
        // assert!(ok);
        //
        // let None = db.next().await else {
        //     panic!("Unexpected event")
        // };

        // db.batch_append(120, vec![event2]).await;
        //
        // let ok = db.try_process_mature_events(5).await;
        // assert!(ok);
        //
        // let Some((sn, export_event_2)) = db.next().await else {
        //     panic!("No event")
        // };
        //
        // //db.delete(sn).await;
        // info!("{:?}", export_event_2);
        // assert_eq!(export_event_2.account_cred, account);
        // assert_eq!(export_event_2.update.share, r2);

        // db.batch_append(150, vec![event3]).await;
        // db.batch_append(160, vec![]).await;

        let ok = db.try_process_mature_events(5).await;
        assert!(ok);

        let Some((sn, export_event_2)) = db.next().await else {
            panic!("No event")
        };
        //db.delete(sn).await;
        info!("{:?}", export_event_2);
    }
}
