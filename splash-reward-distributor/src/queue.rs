use async_trait::async_trait;
use rocksdb::{Options, TransactionDB, TransactionDBOptions};
use std::path::Path;
use std::sync::Arc;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum QueueCmd<TaskId, Task> {
    Schedule(TaskId, Task, StrikeTime),
    Cancel(TaskId),
    Done(TaskId),
    AdvanceClocks(u64),
    DowngradeClocks(u64),
}

#[async_trait]
pub trait TaskQueue<TaskId, Task> {
    async fn batch_execute(self, cmds: Vec<QueueCmd<TaskId, Task>>);
    async fn first(self) -> Option<Task>;
}

#[derive(Clone)]
pub struct RocksDB {
    db: Arc<TransactionDB>,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum StrikeTime {
    Ready,
    At(u64),
}

const PENDING: &str = "pending";
const DONE: &str = "done";
const INDEX: &str = "index";

const TABLES: [&str; 3] = [PENDING, DONE, INDEX];

struct PendingKey<TaskId>(StrikeTime, TaskId);
struct DoneKey<TaskId>(TaskId);
struct IndexKey<TaskId>(TaskId);

impl RocksDB {
    pub fn new<P: AsRef<Path>>(db_path: P) -> Self {
        let mut opts = Options::default();
        opts.create_if_missing(true);
        opts.create_missing_column_families(true);
        let db_opts = TransactionDBOptions::default();
        Self {
            db: Arc::new(TransactionDB::open_cf(&opts, &db_opts, db_path, TABLES).unwrap()),
        }
    }
}
