use rocksdb::{Options, TransactionDB, TransactionDBOptions};
use std::marker::PhantomData;
use std::path::Path;
use std::sync::Arc;

#[derive(Clone)]
pub struct TaskQueue<TaskId, Task> {
    db: Arc<TransactionDB>,
    pd: PhantomData<(TaskId, Task)>,
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

impl<TaskId, Task> TaskQueue<TaskId, Task> {
    pub fn new<P: AsRef<Path>>(db_path: P) -> Self {
        let mut opts = Options::default();
        opts.create_if_missing(true);
        opts.create_missing_column_families(true);
        let db_opts = TransactionDBOptions::default();
        Self {
            db: Arc::new(TransactionDB::open_cf(&opts, &db_opts, db_path, TABLES).unwrap()),
            pd: PhantomData,
        }
    }
    
    pub async fn schedule(self, task_id: TaskId, task: Task, strike_time: StrikeTime) where TaskId: Unpin, Task: Unpin {}
}
