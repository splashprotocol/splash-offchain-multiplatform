use rocksdb::{Options, TransactionDB, TransactionDBOptions};
use std::path::Path;
use std::sync::Arc;
use async_trait::async_trait;

#[async_trait]
pub trait TaskQueue<TaskId, Task> {
    async fn schedule(&self, task_id: TaskId, task: Task, strike_time: StrikeTime);
    async fn cancel(&self, task_id: TaskId);
}

#[derive(Clone)]
pub struct RocksDB {
    db: Arc<TransactionDB>,
}

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
