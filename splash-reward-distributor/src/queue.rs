use async_trait::async_trait;
use futures::channel::mpsc;
use futures::executor::block_on;
use futures::{SinkExt, Stream};
use rocksdb::{
    ColumnFamily, IteratorMode, Options, ReadOptions, Transaction, TransactionDB, TransactionDBOptions,
};
use serde::de::DeserializeOwned;
use serde::Serialize;
use std::path::Path;
use std::sync::Arc;
use tokio::task::spawn_blocking;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum QueueCmd<TaskId, Task> {
    Schedule(TaskId, Task, StrikeTime),
    Update(TaskId, Task),
    Cancel(TaskId),
    Done(TaskId),
    AdvanceClocks(u64),
    DowngradeClocks(u64),
}

#[async_trait]
pub trait TaskQueue<TaskId, Task> {
    async fn batch_execute(self, cmds: Vec<QueueCmd<TaskId, Task>>);
    fn pending_stream(self) -> impl Stream<Item = Task> + Unpin;
    fn done_stream(self) -> impl Stream<Item = Task> + Unpin;
}

#[derive(Clone)]
pub struct RocksDB {
    db: Arc<TransactionDB>,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum StrikeTime {
    Ready,
    At(u64),
    In(u64),
}

const PENDING: &str = "pending";
const DONE: &str = "done";
const INDEX: &str = "index";
const CLOCKS: &str = "clocks";

const TABLES: [&str; 4] = [PENDING, DONE, INDEX, CLOCKS];

struct PendingKey<TaskId>(u64, TaskId);
impl<TaskId> PendingKey<TaskId> {
    fn new(strike_time: u64, id: TaskId) -> Self {
        Self(strike_time, id)
    }
    fn as_bytes(&self) -> Vec<u8>
    where
        TaskId: AsRef<[u8]>,
    {
        let mut bf = vec![];
        bf.extend(self.0.to_be_bytes());
        bf.extend(self.1.as_ref());
        bf
    }
}

struct Tables<'a> {
    pending: &'a ColumnFamily,
    done: &'a ColumnFamily,
    index: &'a ColumnFamily,
    clocks: &'a ColumnFamily,
}

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

    fn tables(&self) -> Tables {
        Tables {
            pending: self.db.cf_handle(PENDING).unwrap(),
            done: self.db.cf_handle(DONE).unwrap(),
            index: self.db.cf_handle(INDEX).unwrap(),
            clocks: self.db.cf_handle(CLOCKS).unwrap(),
        }
    }

    fn read_current_time(&self, tx: &Transaction<TransactionDB>) -> Option<u64> {
        let clocks_cf = self.db.cf_handle(CLOCKS).unwrap();
        let mut current_time = tx.iterator_cf_opt(clocks_cf, ReadOptions::default(), IteratorMode::End);
        if let Some(Ok((key, _))) = current_time.next() {
            return Some(<u64>::from_be_bytes(<[u8; 8]>::try_from(key.as_ref()).ok()?));
        }
        None
    }
}

#[async_trait]
impl<TaskId, Task> TaskQueue<TaskId, Task> for RocksDB
where
    TaskId: Copy + AsRef<[u8]> + Send + 'static,
    Task: Serialize + DeserializeOwned + Send + 'static,
{
    async fn batch_execute(self, cmds: Vec<QueueCmd<TaskId, Task>>) {
        spawn_blocking(move || {
            let tables = self.tables();
            let tx = self.db.transaction();
            for cmd in cmds {
                match cmd {
                    QueueCmd::Schedule(id, task, time) => {
                        let ct = self.read_current_time(&tx).unwrap();
                        schedule(&tx, &tables, id, task, time, ct)
                    }
                    QueueCmd::Update(id, task) => update(&tx, &tables, id, task),
                    QueueCmd::Cancel(id) => cancel(&tx, &tables, id),
                    QueueCmd::Done(id) => done(&tx, &tables, id),
                    QueueCmd::AdvanceClocks(time) => advance_clocks(&tx, &tables, time),
                    QueueCmd::DowngradeClocks(time) => downgrade_clocks(&tx, &tables, time),
                }
            }
            tx.commit().unwrap();
        })
        .await
        .unwrap();
    }

    fn pending_stream(self) -> impl Stream<Item = Task> + Unpin {
        let (mut snd, recv) = mpsc::channel(100);
        spawn_blocking(move || {
            let tables = self.tables();
            let tx = self.db.transaction();
            if let Some(current_time) = self.read_current_time(&tx) {
                let mut pending_tasks =
                    tx.iterator_cf_opt(tables.pending, ReadOptions::default(), IteratorMode::Start);
                while let Some(Ok((key, _))) = pending_tasks.next() {
                    let strike_time_bytes = <[u8; 8]>::try_from(&key[0..8]).unwrap();
                    let strike_time = <u64>::from_be_bytes(strike_time_bytes);
                    if strike_time <= current_time {
                        let task_id = &key[8..];
                        if let Ok(Some(task_bytes)) = tx.get_cf(tables.index, task_id) {
                            if let Ok(None) = tx.get_cf(tables.done, task_id) {
                                let task = rmp_serde::from_slice(&task_bytes).unwrap();
                                if let Err(_) = block_on(snd.send(task)) {
                                    break;
                                }
                                continue;
                            }
                        }
                        tx.delete_cf(tables.pending, task_id).unwrap();
                    }
                }
            }
        });
        recv
    }

    fn done_stream(self) -> impl Stream<Item = Task> + Unpin {
        let (mut snd, recv) = mpsc::channel(100);
        spawn_blocking(move || {
            let tables = self.tables();
            let tx = self.db.transaction();
            let mut done_tasks = tx.iterator_cf_opt(tables.done, ReadOptions::default(), IteratorMode::Start);
            while let Some(Ok((task_id, _))) = done_tasks.next() {
                if let Ok(Some(task_bytes)) = tx.get_cf(tables.index, &task_id) {
                    let task = rmp_serde::from_slice(&task_bytes).unwrap();
                    if let Err(_) = block_on(snd.send(task)) {
                        break;
                    }
                    continue;
                }
                tx.delete_cf(tables.pending, task_id).unwrap();
            }
        });
        recv
    }
}

fn schedule<TaskId: Copy + AsRef<[u8]>, Task: Serialize>(
    tx: &Transaction<TransactionDB>,
    tables: &Tables,
    id: TaskId,
    task: Task,
    time: StrikeTime,
    current_time: u64,
) {
    let exact_strike_time = match time {
        StrikeTime::Ready => current_time,
        StrikeTime::At(t) => t,
        StrikeTime::In(t) => current_time + t,
    };
    let task_bytes = rmp_serde::to_vec(&task).unwrap();
    tx.put_cf(tables.index, id, task_bytes).unwrap();
    tx.put_cf(
        tables.pending,
        PendingKey::new(exact_strike_time, id).as_bytes(),
        &[],
    )
    .unwrap();
}

fn update<TaskId: Copy + AsRef<[u8]>, Task: Serialize>(
    tx: &Transaction<TransactionDB>,
    tables: &Tables,
    id: TaskId,
    task: Task,
) {
    let task_bytes = rmp_serde::to_vec(&task).unwrap();
    tx.put_cf(tables.index, id, task_bytes).unwrap();
}

fn cancel<TaskId: AsRef<[u8]>>(tx: &Transaction<TransactionDB>, tables: &Tables, id: TaskId) {
    tx.delete_cf(tables.index, id).unwrap();
}

fn done<TaskId: Copy + AsRef<[u8]>>(tx: &Transaction<TransactionDB>, tables: &Tables, id: TaskId) {
    tx.put_cf(tables.done, id, []).unwrap();
}

fn advance_clocks(tx: &Transaction<TransactionDB>, tables: &Tables, time: u64) {
    tx.put_cf(tables.clocks, time.to_be_bytes(), []).unwrap()
}

fn downgrade_clocks(tx: &Transaction<TransactionDB>, tables: &Tables, time: u64) {
    tx.delete_cf(tables.clocks, time.to_be_bytes()).unwrap()
}

#[cfg(test)]
mod tests {
    use crate::queue::{QueueCmd, RocksDB, StrikeTime, TaskQueue};
    use futures::StreamExt;
    use serde::{Deserialize, Serialize};
    use splash_testing::db_path::DBPath;
    use std::time::Duration;
    use tokio::time::timeout;

    pub type TaskId = [u8; 32];
    #[derive(Copy, Clone, Eq, PartialEq, Debug, Serialize, Deserialize)]
    pub struct Task(TaskId);

    #[tokio::test]
    async fn execute_schedule() {
        let path = DBPath::new("_test_execute_schedule");
        let db = RocksDB::new(&path);
        let tid0 = [0u8; 32];
        let t0 = Task(tid0);
        db.clone()
            .batch_execute(vec![
                QueueCmd::AdvanceClocks(1),
                QueueCmd::Schedule(tid0, t0, StrikeTime::Ready),
            ])
            .await;
        let ordered_tasks = timeout(
            Duration::from_millis(100),
            <RocksDB as TaskQueue<TaskId, Task>>::pending_stream(db).collect::<Vec<Task>>(),
        )
        .await
        .unwrap();
        assert_eq!(ordered_tasks, vec![t0]);
    }

    #[tokio::test]
    async fn stream_pending_tasks() {
        let path = DBPath::new("_test_stream_pending_tasks");
        let db = RocksDB::new(&path);
        let (tid0, tid1, tid2) = ([0u8; 32], [1u8; 32], [2u8; 32]);
        let (t0, t1, t2) = (Task(tid0), Task(tid1), Task(tid2));
        db.clone()
            .batch_execute(vec![
                QueueCmd::AdvanceClocks(1),
                QueueCmd::Schedule(tid0, t0, StrikeTime::Ready),
            ])
            .await;
        db.clone()
            .batch_execute(vec![
                QueueCmd::AdvanceClocks(2),
                QueueCmd::Schedule(tid1, t1, StrikeTime::In(5)),
            ])
            .await;
        db.clone()
            .batch_execute(vec![
                QueueCmd::AdvanceClocks(4),
                QueueCmd::Schedule(tid2, t2, StrikeTime::Ready),
            ])
            .await;
        let ordered_tasks = timeout(
            Duration::from_millis(100),
            <RocksDB as TaskQueue<TaskId, Task>>::pending_stream(db).collect::<Vec<Task>>(),
        )
        .await
        .unwrap();
        assert_eq!(ordered_tasks, vec![t0, t2]);
    }

    #[tokio::test]
    async fn delete_pending_task() {
        let path = DBPath::new("_test_delete_pending_task");
        let db = RocksDB::new(&path);
        let (tid0, tid1, tid2) = ([0u8; 32], [1u8; 32], [2u8; 32]);
        let (t0, t1, t2) = (Task(tid0), Task(tid1), Task(tid2));
        db.clone()
            .batch_execute(vec![
                QueueCmd::AdvanceClocks(1),
                QueueCmd::Schedule(tid0, t0, StrikeTime::Ready),
            ])
            .await;
        db.clone()
            .batch_execute(vec![
                QueueCmd::AdvanceClocks(2),
                QueueCmd::Schedule(tid1, t1, StrikeTime::In(5)),
            ])
            .await;
        db.clone()
            .batch_execute(vec![
                QueueCmd::AdvanceClocks(4),
                QueueCmd::Schedule(tid2, t2, StrikeTime::Ready),
            ])
            .await;
        db.clone()
            .batch_execute(vec![QueueCmd::<TaskId, Task>::Cancel(tid2)])
            .await;
        let ordered_tasks = timeout(
            Duration::from_millis(100),
            <RocksDB as TaskQueue<TaskId, Task>>::pending_stream(db).collect::<Vec<Task>>(),
        )
        .await
        .unwrap();
        assert_eq!(ordered_tasks, vec![t0]);
    }

    #[tokio::test]
    async fn mark_task_done() {
        let path = DBPath::new("_test_mark_task_done");
        let db = RocksDB::new(&path);
        let (tid0, tid1, tid2) = ([0u8; 32], [1u8; 32], [2u8; 32]);
        let (t0, t1, t2) = (Task(tid0), Task(tid1), Task(tid2));
        db.clone()
            .batch_execute(vec![
                QueueCmd::AdvanceClocks(1),
                QueueCmd::Schedule(tid0, t0, StrikeTime::Ready),
            ])
            .await;
        db.clone()
            .batch_execute(vec![
                QueueCmd::AdvanceClocks(2),
                QueueCmd::Schedule(tid1, t1, StrikeTime::In(5)),
            ])
            .await;
        db.clone()
            .batch_execute(vec![
                QueueCmd::AdvanceClocks(4),
                QueueCmd::Schedule(tid2, t2, StrikeTime::Ready),
            ])
            .await;
        db.clone()
            .batch_execute(vec![QueueCmd::<TaskId, Task>::Done(tid2)])
            .await;
        let ordered_pending_tasks = timeout(
            Duration::from_millis(100),
            <RocksDB as TaskQueue<TaskId, Task>>::pending_stream(db.clone()).collect::<Vec<Task>>(),
        )
        .await
        .unwrap();
        assert_eq!(ordered_pending_tasks, vec![t0]);
        let ordered_done_tasks = timeout(
            Duration::from_millis(100),
            <RocksDB as TaskQueue<TaskId, Task>>::done_stream(db).collect::<Vec<Task>>(),
        )
        .await
        .unwrap();
        assert_eq!(ordered_done_tasks, vec![t2]);
    }

    #[tokio::test]
    async fn update_task() {
        let path = DBPath::new("_test_update_task");
        let db = RocksDB::new(&path);
        let (tid0, tid1) = ([0u8; 32], [1u8; 32]);
        let (t0, t1) = (Task(tid0), Task(tid1));
        let t1_upd = Task([3u8; 32]);
        db.clone()
            .batch_execute(vec![
                QueueCmd::AdvanceClocks(1),
                QueueCmd::Schedule(tid0, t0, StrikeTime::Ready),
            ])
            .await;
        db.clone()
            .batch_execute(vec![
                QueueCmd::AdvanceClocks(2),
                QueueCmd::Schedule(tid1, t1, StrikeTime::In(5)),
            ])
            .await;
        db.clone()
            .batch_execute(vec![QueueCmd::AdvanceClocks(7), QueueCmd::Update(tid1, t1_upd)])
            .await;
        let ordered_pending_tasks = timeout(
            Duration::from_millis(100),
            <RocksDB as TaskQueue<TaskId, Task>>::pending_stream(db.clone()).collect::<Vec<Task>>(),
        )
        .await
        .unwrap();
        assert_eq!(ordered_pending_tasks, vec![t0, t1_upd]);
    }
}
