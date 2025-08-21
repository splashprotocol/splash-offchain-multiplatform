use async_trait::async_trait;
use cml_crypto::{RawBytesEncoding, TransactionHash};
use futures::channel::mpsc;
use futures::executor::block_on;
use futures::{SinkExt, Stream};
use rocksdb::{
    ColumnFamily, Direction, IteratorMode, Options, ReadOptions, Transaction, TransactionDB,
    TransactionDBOptions,
};
use serde::de::DeserializeOwned;
use serde::Serialize;
use std::fmt::Debug;
use std::path::Path;
use std::sync::Arc;
use tokio::task::spawn_blocking;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum QueueCmd<TaskId, Task> {
    Schedule(TaskId, Task, StrikeTime),
    Update(TaskId, Task),
    Cancel(TaskId),
    Done(TaskId, TransactionHash),
    AdvanceClocks(u64),
    DowngradeClocks(u64),
}

#[async_trait]
pub trait TaskQueue<TaskId, Task> {
    async fn batch_execute(self, cmds: Vec<QueueCmd<TaskId, Task>>);
    /// Removes all Done task_ids associated with the given TX hash from the index and returns them.
    async fn drop_tx(self, tx_hash: TransactionHash) -> Option<Vec<TaskId>>;
    fn pending_stream(self) -> impl Stream<Item = (TaskId, Task)> + Unpin + Send;
    fn done_stream(self) -> impl Stream<Item = (TaskId, Task)> + Unpin + Send;
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum StrikeTime {
    Ready,
    At(u64),
    In(u64),
}

const PENDING: &str = "pending";
const DONE: &str = "done";
const DONE_WITH_TX_HASH: &str = "done_tx_hash";
const INDEX: &str = "index";
const CLOCKS: &str = "clocks";
const DONE_TASK_ID_IX_START: usize = 32;

const TABLES: [&str; 5] = [PENDING, DONE, DONE_WITH_TX_HASH, INDEX, CLOCKS];

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
    /// Maps [tx_hash|task_id] to empty slice []
    done_with_tx_hash: &'a ColumnFamily,
    done: &'a ColumnFamily,
    index: &'a ColumnFamily,
    clocks: &'a ColumnFamily,
}

#[derive(Clone)]
pub struct RocksDB {
    db: Arc<TransactionDB>,
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
            done_with_tx_hash: self.db.cf_handle(DONE_WITH_TX_HASH).unwrap(),
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
    TaskId: Copy + TryFrom<Vec<u8>> + AsRef<[u8]> + Send + Serialize + DeserializeOwned + PartialEq + 'static,
    TaskId::Error: Debug,
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
                    QueueCmd::Done(id, tx_hash) => {
                        done(&tx, &tables, tx_hash, id);
                    }
                    QueueCmd::AdvanceClocks(time) => advance_clocks(&tx, &tables, time),
                    QueueCmd::DowngradeClocks(time) => downgrade_clocks(&tx, &tables, time),
                }
            }

            tx.commit().unwrap();
        })
        .await
        .unwrap();
    }

    async fn drop_tx(self, tx_hash: TransactionHash) -> Option<Vec<TaskId>> {
        spawn_blocking(move || {
            let tx = self.db.transaction();
            let tables = self.tables();
            let mut readopts = ReadOptions::default();
            let prefix = tx_hash.to_raw_bytes();
            readopts.set_iterate_range(rocksdb::PrefixRange(prefix));
            let mut to_delete = vec![];
            let mut task_ids = vec![];
            {
                let mut done_tasks = tx.iterator_cf_opt(
                    &tables.done_with_tx_hash,
                    readopts,
                    IteratorMode::From(prefix, Direction::Forward),
                );
                while let Some(Ok((key_with_tx_hash, _))) = done_tasks.next() {
                    let task_id_bytes = key_with_tx_hash[DONE_TASK_ID_IX_START..].to_vec();
                    let task_id = TaskId::try_from(task_id_bytes.clone()).unwrap();
                    to_delete.push((task_id_bytes, key_with_tx_hash.to_vec()));
                    task_ids.push(task_id);
                }

                for (task_id_key, key_with_tx_hash) in to_delete {
                    tx.delete_cf(&tables.done, task_id_key).unwrap();
                    tx.delete_cf(&tables.done_with_tx_hash, key_with_tx_hash).unwrap();
                }
            }

            tx.commit().unwrap();
            if !task_ids.is_empty() {
                Some(task_ids)
            } else {
                None
            }
        })
        .await
        .unwrap()
    }

    fn pending_stream(self) -> impl Stream<Item = (TaskId, Task)> + Unpin {
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
                                let task_id = task_id.to_vec().try_into().ok().unwrap();
                                let task = rmp_serde::from_slice(&task_bytes).unwrap();
                                if block_on(snd.send((task_id, task))).is_err() {
                                    break;
                                }
                                continue;
                            }
                        }
                        tx.delete_cf(tables.pending, task_id).unwrap();
                    }
                }
            }
            tx.commit().unwrap();
        });
        recv
    }

    fn done_stream(self) -> impl Stream<Item = (TaskId, Task)> + Unpin {
        let (mut snd, recv) = mpsc::channel(100);
        spawn_blocking(move || {
            let tables = self.tables();
            let tx = self.db.transaction();
            {
                let mut done_tasks = tx.iterator_cf_opt(
                    tables.done_with_tx_hash,
                    ReadOptions::default(),
                    IteratorMode::Start,
                );
                while let Some(Ok((key_with_tx_hash, _))) = done_tasks.next() {
                    let task_id = &key_with_tx_hash[DONE_TASK_ID_IX_START..];
                    if let Ok(Some(task_bytes)) = tx.get_cf(tables.index, task_id) {
                        let task_id = task_id.to_vec().try_into().ok().unwrap();
                        let task = rmp_serde::from_slice(&task_bytes).unwrap();
                        if block_on(snd.send((task_id, task))).is_err() {
                            break;
                        }
                        continue;
                    }
                    tx.delete_cf(tables.done, task_id).unwrap();
                    tx.delete_cf(tables.done_with_tx_hash, key_with_tx_hash).unwrap();
                }
            }
            tx.commit().unwrap();
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
        [],
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

fn done<TaskId: Copy + AsRef<[u8]>>(
    tx: &Transaction<TransactionDB>,
    tables: &Tables,
    tx_hash: TransactionHash,
    id: TaskId,
) {
    let mut key = tx_hash.to_raw_bytes().to_vec();
    key.extend_from_slice(id.as_ref());
    tx.put_cf(tables.done, id, []).unwrap();
    tx.put_cf(tables.done_with_tx_hash, key, []).unwrap();
}

fn advance_clocks(tx: &Transaction<TransactionDB>, tables: &Tables, time: u64) {
    tx.put_cf(tables.clocks, time.to_be_bytes(), []).unwrap()
}

fn downgrade_clocks(tx: &Transaction<TransactionDB>, tables: &Tables, time: u64) {
    tx.delete_cf(tables.clocks, time.to_be_bytes()).unwrap()
}

#[cfg(test)]
mod tests {
    use crate::engine::queue::{QueueCmd, RocksDB, StrikeTime, TaskQueue};
    use cml_crypto::{RawBytesEncoding, TransactionHash};
    use futures::StreamExt;
    use serde::{Deserialize, Serialize};
    use splash_testing::db_path::DBPath;
    use std::time::Duration;
    use tokio::time::timeout;

    #[derive(Copy, Clone, Eq, PartialEq, Debug, Serialize, Deserialize)]
    pub struct TaskId([u8; 32]);
    impl AsRef<[u8]> for TaskId {
        fn as_ref(&self) -> &[u8] {
            self.0.as_ref()
        }
    }
    impl TryFrom<Vec<u8>> for TaskId {
        type Error = ();
        fn try_from(value: Vec<u8>) -> Result<Self, Self::Error> {
            <[u8; 32]>::try_from(&*value).map(Self).map_err(|_| ())
        }
    }
    #[derive(Copy, Clone, Eq, PartialEq, Debug, Serialize, Deserialize)]
    pub struct Task(TaskId);

    #[tokio::test]
    async fn execute_schedule() {
        let path = DBPath::new("_test_execute_schedule");
        let db = RocksDB::new(&path);
        let tid0 = TaskId([0u8; 32]);
        let t0 = Task(tid0);
        let cmds: Vec<QueueCmd<_, _>> = vec![
            QueueCmd::AdvanceClocks(1),
            QueueCmd::Schedule(tid0, t0, StrikeTime::Ready),
        ];
        db.clone().batch_execute(cmds).await;
        let ordered_tasks = timeout(
            Duration::from_millis(100),
            <RocksDB as TaskQueue<TaskId, Task>>::pending_stream(db).collect::<Vec<(TaskId, Task)>>(),
        )
        .await
        .unwrap();
        assert_eq!(
            ordered_tasks.into_iter().map(|(_, t)| t).collect::<Vec<_>>(),
            vec![t0]
        );
    }

    #[tokio::test]
    async fn stream_pending_tasks() {
        let path = DBPath::new("_test_stream_pending_tasks");
        let db = RocksDB::new(&path);
        let (tid0, tid1, tid2) = (TaskId([0u8; 32]), TaskId([1u8; 32]), TaskId([2u8; 32]));
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
            <RocksDB as TaskQueue<TaskId, Task>>::pending_stream(db).collect::<Vec<(TaskId, Task)>>(),
        )
        .await
        .unwrap();
        assert_eq!(
            ordered_tasks.into_iter().map(|(_, t)| t).collect::<Vec<_>>(),
            vec![t0, t2]
        );
    }

    #[tokio::test]
    async fn delete_pending_task() {
        let path = DBPath::new("_test_delete_pending_task");
        let db = RocksDB::new(&path);
        let (tid0, tid1, tid2) = (TaskId([0u8; 32]), TaskId([1u8; 32]), TaskId([2u8; 32]));
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
            <RocksDB as TaskQueue<TaskId, Task>>::pending_stream(db).collect::<Vec<(TaskId, Task)>>(),
        )
        .await
        .unwrap();
        assert_eq!(
            ordered_tasks.into_iter().map(|(_, t)| t).collect::<Vec<_>>(),
            vec![t0]
        );
    }

    #[tokio::test]
    async fn mark_task_done() {
        let path = DBPath::new("_test_mark_task_done");
        let db = RocksDB::new(&path);
        let (tid0, tid1, tid2) = (TaskId([0u8; 32]), TaskId([1u8; 32]), TaskId([2u8; 32]));
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
        let tx_hash = TransactionHash::from_raw_bytes(&[0; 32]).unwrap();
        db.clone()
            .batch_execute(vec![QueueCmd::<TaskId, Task>::Done(tid2, tx_hash)])
            .await;
        let ordered_pending_tasks = timeout(
            Duration::from_millis(100),
            <RocksDB as TaskQueue<TaskId, Task>>::pending_stream(db.clone()).collect::<Vec<(TaskId, Task)>>(),
        )
        .await
        .unwrap();
        assert_eq!(
            ordered_pending_tasks
                .into_iter()
                .map(|(_, t)| t)
                .collect::<Vec<_>>(),
            vec![t0]
        );
        let ordered_done_tasks = timeout(
            Duration::from_millis(100),
            <RocksDB as TaskQueue<TaskId, Task>>::done_stream(db).collect::<Vec<(TaskId, Task)>>(),
        )
        .await
        .unwrap();
        assert_eq!(
            ordered_done_tasks.into_iter().map(|(_, t)| t).collect::<Vec<_>>(),
            vec![t2]
        );
    }

    #[tokio::test]
    async fn update_task() {
        let path = DBPath::new("_test_update_task");
        let db = RocksDB::new(&path);
        let (tid0, tid1) = (TaskId([0u8; 32]), TaskId([1u8; 32]));
        let (t0, t1) = (Task(tid0), Task(tid1));
        let t1_upd = Task(TaskId([3u8; 32]));
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
            <RocksDB as TaskQueue<TaskId, Task>>::pending_stream(db.clone()).collect::<Vec<(TaskId, Task)>>(),
        )
        .await
        .unwrap();
        assert_eq!(
            ordered_pending_tasks
                .into_iter()
                .map(|(_, t)| t)
                .collect::<Vec<_>>(),
            vec![t0, t1_upd]
        );
    }

    #[tokio::test]
    async fn test_task_ids_by_tx_hash() {
        let path = DBPath::new("_test_mark_task_done");
        let db = RocksDB::new(&path);
        let (tid0, tid1, tid2) = (TaskId([0u8; 32]), TaskId([1u8; 32]), TaskId([2u8; 32]));
        let tx_hash = TransactionHash::from_raw_bytes(&[0; 32]).unwrap();
        let cmds = vec![
            QueueCmd::<TaskId, Task>::Done(tid0, tx_hash),
            QueueCmd::Done(tid1, tx_hash),
            QueueCmd::Done(tid2, tx_hash),
        ];

        db.clone().batch_execute(cmds).await;
        let result: Vec<TaskId> = <RocksDB as TaskQueue<TaskId, Task>>::drop_tx(db, tx_hash)
            .await
            .unwrap();

        assert_eq!(result, vec![tid0, tid1, tid2]);
    }
}
