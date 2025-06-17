use std::collections::{HashMap, VecDeque};
use std::fmt::{Debug, Display};
use std::hash::Hash;
use std::sync::Arc;

use async_trait::async_trait;
use bounded_integer::BoundedU8;
use chrono::{Duration, Utc};
use derive_more::{From, Into};
use log::trace;
use priority_queue::PriorityQueue;
use rand::Rng;
use serde::{Deserialize, Serialize};
use serde_with::serde_as;
use tokio::sync::Mutex;
use type_equalities::IsEqual;

use crate::backlog::data::{BacklogOrder, OrderWeight, Weighted};
use crate::backlog::persistence::BacklogStore;
use crate::data::circular_filter::CircularFilter;
use crate::display::display_option;
use crate::domain::order::{PendingOrder, ProgressingOrder, SpecializedOrder, SuspendedOrder, UniqueOrder};
use crate::domain::Has;
use crate::maker::Maker;
use crate::tracing::Tracing;

pub mod data;
pub mod persistence;

/// A buffer for "hot" orders. Doesn't care about resiliency.
pub trait HotBacklog<TOrd>
where
    TOrd: UniqueOrder,
{
    /// Add new pending order to backlog.
    fn put<'a>(&mut self, ord: TOrd)
    where
        TOrd: 'a;
    /// Pop best order.
    fn try_pop(&mut self) -> Option<TOrd>;
    /// Check if order with the given id exists already in backlog.
    fn exists<'a>(&self, ord_id: TOrd::TOrderId) -> bool
    where
        TOrd::TOrderId: 'a;
    /// Remove order from backlog.
    fn remove<'a>(&mut self, ord_id: TOrd::TOrderId)
    where
        TOrd::TOrderId: 'a + Clone;
    /// Check order later.
    fn soft_evict<'a>(&mut self, ord: TOrd::TOrderId)
    where
        TOrd: 'a;
}

impl<In, T> HotBacklog<T> for Tracing<In>
where
    In: HotBacklog<T>,
    T: UniqueOrder,
    T::TOrderId: Display,
{
    fn put<'a>(&mut self, ord: T)
    where
        T: 'a,
    {
        trace!("HotBacklog::put({})", ord.get_self_ref());
        self.component.put(ord);
    }

    fn try_pop(&mut self) -> Option<T> {
        let res = self.component.try_pop();
        trace!(
            "HotBacklog::try_pop() -> {}",
            display_option(&res.as_ref().map(|o| o.get_self_ref()))
        );
        res
    }

    fn exists<'a>(&self, id: T::TOrderId) -> bool
    where
        T::TOrderId: 'a,
    {
        let res = self.component.exists(id);
        trace!("HotBacklog::exists({}) -> {}", id, res);
        res
    }

    fn remove<'a>(&mut self, id: T::TOrderId)
    where
        T::TOrderId: 'a + Clone,
    {
        trace!("HotBacklog::remove({})", id);
        self.component.remove(id);
    }

    fn soft_evict<'a>(&mut self, id: T::TOrderId)
    where
        T: 'a,
    {
        trace!("HotBacklog::soft_evict({})", id);
        self.component.soft_evict(id);
    }
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Into, From)]
pub struct BacklogCapacity(u32);

#[derive(Clone)]
pub struct HotPriorityBacklog<TOrd: UniqueOrder> {
    queue: PriorityQueue<TOrd::TOrderId, OrderWeight>,
    store: HashMap<TOrd::TOrderId, TOrd>,
    soft_evicted_orders: CircularFilter<256, TOrd::TOrderId>,
    capacity: u32,
}

impl<TOrd: UniqueOrder> HotPriorityBacklog<TOrd> {
    pub fn new(capacity: BacklogCapacity) -> Self {
        Self {
            queue: PriorityQueue::new(),
            store: HashMap::new(),
            soft_evicted_orders: CircularFilter::new(),
            capacity: capacity.into(),
        }
    }
}

impl<K, Ctx, TOrd> Maker<K, Ctx> for HotPriorityBacklog<TOrd>
where
    TOrd: SpecializedOrder,
    Ctx: Has<BacklogCapacity>,
{
    fn make(_: K, ctx: &Ctx) -> Self {
        HotPriorityBacklog::new(ctx.get())
    }
}

impl<TOrd> HotBacklog<TOrd> for HotPriorityBacklog<TOrd>
where
    TOrd: UniqueOrder + Weighted + Hash + Eq + Clone,
    TOrd::TOrderId: Copy,
{
    fn put<'a>(&mut self, ord: TOrd)
    where
        TOrd: 'a,
    {
        let id = ord.get_self_ref();
        if self.capacity > 0 && !self.store.contains_key(&id) && !self.soft_evicted_orders.contains(&id) {
            let wt = ord.weight();
            self.queue.push(id, wt);
            self.store.insert(id, ord);
            self.capacity -= 1;
        }
    }

    fn try_pop(&mut self) -> Option<TOrd> {
        while let Some((oid, _)) = self.queue.pop() {
            if let Some(ord) = self.store.remove(&oid) {
                self.capacity += 1;
                return Some(ord);
            }
        }
        None
    }

    fn exists<'a>(&self, ord_id: TOrd::TOrderId) -> bool
    where
        TOrd::TOrderId: 'a,
    {
        self.store.contains_key(&ord_id)
    }

    fn remove<'a>(&mut self, ord: TOrd::TOrderId)
    where
        TOrd::TOrderId: 'a + Clone,
    {
        self.soft_evicted_orders.remove(&ord);
        if self.store.remove(&ord).is_some() {
            self.capacity += 1;
        }
    }

    fn soft_evict<'a>(&mut self, ord: TOrd::TOrderId)
    where
        TOrd: 'a,
    {
        self.soft_evicted_orders.add(ord);
    }
}

/// Backlog manages orders on all stages of their life.
/// Usually in the order defined by some weighting function (e.g. orders with higher fee are preferred).
#[async_trait]
pub trait ResilientBacklog<TOrd>
where
    TOrd: UniqueOrder,
{
    /// Add new pending order to backlog.
    async fn put<'a>(&self, ord: PendingOrder<TOrd>)
    where
        TOrd: 'a;
    /// Suspend order that temporarily failed.
    /// Potentially retry later.
    async fn suspend<'a>(&self, ord: TOrd) -> bool
    where
        TOrd: 'a;
    /// Register successful order to check if it settled later.
    async fn check_later<'a>(&self, ord: ProgressingOrder<TOrd>) -> bool
    where
        TOrd: 'a;
    /// Pop best order.
    async fn try_pop(&self) -> Option<TOrd>;
    /// Check if order with the given id exists already in backlog.
    async fn exists<'a>(&self, ord_id: TOrd::TOrderId) -> bool
    where
        TOrd::TOrderId: 'a;
    /// Remove order from backlog.
    async fn remove<'a>(&self, ord_id: TOrd::TOrderId)
    where
        TOrd::TOrderId: 'a + Clone;
    /// Return order back to backlog.
    async fn recharge<'a>(&self, ord: TOrd)
    where
        TOrd: 'a;
    /// Return all orders satisfying the given predicate.
    async fn find_orders<F: Fn(&TOrd) -> bool + Send + 'static>(&self, f: F) -> Vec<TOrd>
    where
        F: Fn(&TOrd) -> bool + Send + 'static;
}

#[async_trait]
impl<TOrd, B> ResilientBacklog<TOrd> for Tracing<B>
where
    TOrd: UniqueOrder + Debug + Clone,
    TOrd::TOrderId: Debug + Clone + Send + Sync,
    TOrd: Send + Sync,
    B: ResilientBacklog<TOrd> + Send + Sync,
{
    async fn put<'a>(&self, ord: PendingOrder<TOrd>)
    where
        TOrd: 'a,
    {
        trace!(target: "backlog", "put({:?})", ord);
        self.component.put(ord.clone()).await;
        trace!(target: "backlog", "put({:?}) -> ()", ord);
    }

    async fn suspend<'a>(&self, ord: TOrd) -> bool
    where
        TOrd: 'a,
    {
        trace!(target: "backlog", "suspend({:?})", ord);
        let res = self.component.suspend(ord.clone()).await;
        trace!(target: "backlog", "suspend({:?}) -> {:?}", ord, res);
        res
    }

    async fn check_later<'a>(&self, ord: ProgressingOrder<TOrd>) -> bool
    where
        TOrd: 'a,
    {
        trace!(target: "backlog", "check_later({:?})", ord);
        let res = self.component.check_later(ord.clone()).await;
        trace!(target: "backlog", "check_later({:?}) -> {:?}", ord, res);
        res
    }

    async fn try_pop(&self) -> Option<TOrd> {
        trace!(target: "backlog", "try_pop()");
        let res = self.component.try_pop().await;
        trace!(target: "backlog", "try_pop() -> {:?}", res);
        res
    }

    async fn exists<'a>(&self, ord_id: TOrd::TOrderId) -> bool
    where
        TOrd::TOrderId: 'a,
    {
        self.component.exists(ord_id.clone()).await
    }

    async fn remove<'a>(&self, ord_id: TOrd::TOrderId)
    where
        TOrd::TOrderId: 'a,
    {
        trace!(target: "backlog", "remove({:?})", ord_id);
        self.component.remove(ord_id.clone()).await;
        trace!(target: "backlog", "remove({:?}) -> ()", ord_id);
    }

    async fn recharge<'a>(&self, ord: TOrd)
    where
        TOrd: 'a,
    {
        trace!(target: "backlog", "recharge({:?})", ord);
        self.component.recharge(ord.clone()).await;
        trace!(target: "backlog", "recharge({:?}) -> ()", ord);
    }

    async fn find_orders<F>(&self, f: F) -> Vec<TOrd>
    where
        F: Fn(&TOrd) -> bool + Send + 'static,
    {
        trace!(target: "backlog", "find_order()");
        let res = self.component.find_orders(f).await;
        trace!(target: "backlog", "find_order() -> {:?}", res);
        res
    }
}

#[serde_with::serde_as]
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct BacklogConfig {
    #[serde_as(as = "serde_with::DurationSeconds<i64>")]
    pub order_lifespan: Duration,
    #[serde_as(as = "serde_with::DurationSeconds<i64>")]
    pub order_exec_time: Duration,
    pub retry_suspended_prob: BoundedU8<0, 100>,
}

#[derive(Clone, Debug, Ord, PartialOrd, Eq, PartialEq, Hash)]
struct WeightedOrder<TOrder> {
    order: TOrder,
    timestamp: i64,
}

impl<TOrd> From<&BacklogOrder<TOrd>> for WeightedOrder<TOrd::TOrderId>
where
    TOrd: UniqueOrder,
{
    fn from(bo: &BacklogOrder<TOrd>) -> Self {
        Self {
            order: bo.order.get_self_ref(),
            timestamp: bo.timestamp,
        }
    }
}

impl<TOrd> From<&PendingOrder<TOrd>> for WeightedOrder<TOrd::TOrderId>
where
    TOrd: UniqueOrder,
{
    fn from(po: &PendingOrder<TOrd>) -> Self {
        Self {
            order: po.order.get_self_ref(),
            timestamp: po.timestamp,
        }
    }
}

impl<TOrd> From<ProgressingOrder<TOrd>> for WeightedOrder<TOrd::TOrderId>
where
    TOrd: UniqueOrder,
{
    fn from(po: ProgressingOrder<TOrd>) -> Self {
        Self {
            order: po.order.get_self_ref(),
            timestamp: po.timestamp,
        }
    }
}

impl<TOrd> From<SuspendedOrder<TOrd>> for WeightedOrder<TOrd::TOrderId>
where
    TOrd: UniqueOrder,
{
    fn from(so: SuspendedOrder<TOrd>) -> Self {
        Self {
            order: so.order.get_self_ref(),
            timestamp: so.timestamp,
        }
    }
}

struct InMemoryState<TOrd: UniqueOrder> {
    /// Pending orders ordered by weight.
    pending_pq: PriorityQueue<WeightedOrder<TOrd::TOrderId>, OrderWeight>,
    /// Failed orders waiting for retry (retries are performed with some constant probability, e.g. 5%).
    /// Again, ordered by weight.
    suspended_pq: PriorityQueue<WeightedOrder<TOrd::TOrderId>, OrderWeight>,
    /// Successfully submitted orders. Left orders should be re-executed in some time.
    /// Normally successful orders are eliminated from this queue before new execution attempt.
    revisit_queue: VecDeque<WeightedOrder<TOrd::TOrderId>>,
}

pub struct PersistentPriorityBacklog<TOrd, TStore>
where
    TOrd: UniqueOrder + Hash + Eq,
{
    store: TStore,
    conf: BacklogConfig,
    state: Arc<Mutex<InMemoryState<TOrd>>>,
}

impl<TOrd, TStore> PersistentPriorityBacklog<TOrd, TStore>
where
    TOrd: UniqueOrder + Weighted + Hash + Eq,
    TOrd::TOrderId: Debug,
    TStore: BacklogStore<TOrd>,
{
    pub async fn new<TOrd0: IsEqual<TOrd>>(store: TStore, conf: BacklogConfig) -> Self {
        let mut pending_pq = PriorityQueue::new();
        for ord in store.find_orders(|_| true).await {
            let wt = ord.order.weight();
            trace!(target: "backlog", "Restored order: {:?}", ord.order.get_self_ref());
            pending_pq.push((&ord).into(), wt);
        }
        Self {
            store,
            conf,
            state: Arc::new(Mutex::new(InMemoryState {
                pending_pq,
                suspended_pq: PriorityQueue::new(),
                revisit_queue: VecDeque::new(),
            })),
        }
    }

    async fn revisit_progressing_orders(&self) {
        let mut too_recent_order = None;
        let mut state = self.state.lock().await;
        while let Some(ord) = state.revisit_queue.pop_front() {
            let ts_now = Utc::now().timestamp();
            let elapsed_secs = ts_now - ord.timestamp;
            if elapsed_secs > self.conf.order_exec_time.num_seconds() {
                if elapsed_secs <= self.conf.order_lifespan.num_seconds() {
                    if let Some(ord) = self.store.get(ord.order).await {
                        let wt = ord.order.weight();
                        state.pending_pq.push((&ord).into(), wt);
                    }
                } else {
                    self.store.remove(ord.order).await;
                }
            } else {
                // Too soon to consider `ord`, return it to queue.
                too_recent_order = Some(ord);
                break;
            }
        }

        if let Some(ord) = too_recent_order {
            state.revisit_queue.push_front(ord);
        }
    }
}

#[async_trait]
impl<TOrd, TStore> ResilientBacklog<TOrd> for PersistentPriorityBacklog<TOrd, TStore>
where
    TStore: BacklogStore<TOrd> + Send + Sync,
    TOrd: Send + Sync,
    TOrd::TOrderId: Debug + Clone + Send + Sync,
    TOrd: UniqueOrder + Weighted + Hash + Eq + Clone,
{
    async fn put<'a>(&self, ord: PendingOrder<TOrd>)
    where
        TOrd: 'a,
    {
        if !self.store.exists(ord.order.get_self_ref()).await {
            self.store
                .put(BacklogOrder {
                    order: ord.order.clone(),
                    timestamp: ord.timestamp,
                })
                .await;
        }

        let mut st = self.state.lock().await;
        if let Some(index) = st
            .revisit_queue
            .iter()
            .position(|wo| wo.order == ord.order.get_self_ref())
        {
            st.revisit_queue.remove(index);
        }

        if let Some(element) = st
            .suspended_pq
            .iter()
            .find(|wo| wo.0.order == ord.order.get_self_ref())
            .map(|(e, _)| e)
            .cloned()
        {
            st.suspended_pq.remove(&element);
        }

        if !st
            .pending_pq
            .iter()
            .any(|wo| wo.0.order == ord.order.get_self_ref())
        {
            let wt = ord.order.weight();
            st.pending_pq.push((&ord).into(), wt);
        }
    }

    async fn suspend<'a>(&self, ord: TOrd) -> bool
    where
        TOrd: 'a,
    {
        if self.store.exists(ord.get_self_ref()).await {
            let wt = ord.weight();
            if let Some(backlog_ord) = self.store.get(ord.get_self_ref()).await {
                self.state.lock().await.suspended_pq.push(
                    WeightedOrder {
                        order: ord.get_self_ref(),
                        timestamp: backlog_ord.timestamp,
                    },
                    wt,
                );
                return true;
            }
        }
        false
    }

    async fn check_later<'a>(&self, ord: ProgressingOrder<TOrd>) -> bool
    where
        TOrd: 'a,
    {
        if self.store.exists(ord.order.get_self_ref()).await {
            self.state.lock().await.revisit_queue.push_back(ord.into());
            return true;
        }
        false
    }

    async fn try_pop(&self) -> Option<TOrd> {
        self.revisit_progressing_orders().await;
        let rng = rand::thread_rng().gen_range(0..=99);
        if rng >= self.conf.retry_suspended_prob.get() {
            try_pop_max_order(&self.conf, &self.store, &mut self.state.lock().await.pending_pq).await
        } else {
            try_pop_max_order(&self.conf, &self.store, &mut self.state.lock().await.suspended_pq).await
        }
    }

    async fn exists<'a>(&self, ord_id: TOrd::TOrderId) -> bool
    where
        TOrd::TOrderId: 'a,
    {
        self.store.exists(ord_id).await
    }

    async fn remove<'a>(&self, ord_id: TOrd::TOrderId)
    where
        TOrd::TOrderId: Clone + 'a,
    {
        self.store.remove(ord_id).await;
    }

    async fn recharge<'a>(&self, ord: TOrd)
    where
        TOrd: 'a,
    {
        let wt = ord.weight();
        if let Some(backlog_ord) = self.store.get(ord.get_self_ref()).await {
            self.state.lock().await.pending_pq.push(
                WeightedOrder {
                    order: ord.get_self_ref(),
                    timestamp: backlog_ord.timestamp,
                },
                wt,
            );
        }
    }

    async fn find_orders<F>(&self, f: F) -> Vec<TOrd>
    where
        F: Fn(&TOrd) -> bool + Send + 'static,
    {
        self.store
            .find_orders(f)
            .await
            .into_iter()
            .map(|b| b.order)
            .collect()
    }
}

async fn try_pop_max_order<TOrd, TStore>(
    conf: &BacklogConfig,
    store: &TStore,
    pq: &mut PriorityQueue<WeightedOrder<TOrd::TOrderId>, OrderWeight>,
) -> Option<TOrd>
where
    TOrd: UniqueOrder + Weighted + Hash + Eq,
    TStore: BacklogStore<TOrd>,
{
    while let Some((ord, _)) = pq.pop() {
        let ts_now = Utc::now().timestamp();
        let elapsed_secs = ts_now - ord.timestamp;
        if elapsed_secs > conf.order_lifespan.num_seconds() {
            store.remove(ord.order).await;
        } else {
            let res = store.get(ord.order).await.map(|bo| bo.order);
            if res.is_some() {
                return res;
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::fmt::{Display, Formatter};
    use std::sync::Arc;

    use async_trait::async_trait;
    use bounded_integer::BoundedU8;
    use chrono::{Duration, Utc};
    use rand::RngCore;
    use serde::{Deserialize, Serialize};
    use tokio::sync::Mutex;

    use crate::backlog::data::{BacklogOrder, OrderWeight, Weighted};
    use crate::backlog::persistence::{BacklogStore, BacklogStoreRocksDB};
    use crate::backlog::{BacklogConfig, PersistentPriorityBacklog, ResilientBacklog};
    use crate::domain::order::{PendingOrder, ProgressingOrder, SuspendedOrder, UniqueOrder};

    #[derive(Debug, Ord, PartialOrd, Eq, PartialEq, Hash, Clone, Copy, Serialize, Deserialize)]
    struct MockOrderId(i64);

    impl Display for MockOrderId {
        fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
            f.write_str(format!("{}", self.0.to_string()).as_str())
        }
    }

    #[derive(Debug, Ord, PartialOrd, Eq, PartialEq, Hash, Clone, Serialize, Deserialize)]
    struct MockOrder {
        order_id: MockOrderId,
        weight: OrderWeight,
    }

    impl From<BacklogOrder<MockOrder>> for PendingOrder<MockOrder> {
        fn from(x: BacklogOrder<MockOrder>) -> Self {
            Self {
                order: x.order,
                timestamp: x.timestamp,
            }
        }
    }

    impl From<BacklogOrder<MockOrder>> for ProgressingOrder<MockOrder> {
        fn from(x: BacklogOrder<MockOrder>) -> Self {
            Self {
                order: x.order,
                timestamp: x.timestamp,
            }
        }
    }

    impl From<BacklogOrder<MockOrder>> for SuspendedOrder<MockOrder> {
        fn from(x: BacklogOrder<MockOrder>) -> Self {
            Self {
                order: x.order,
                timestamp: x.timestamp,
            }
        }
    }

    impl Weighted for MockOrder {
        fn weight(&self) -> OrderWeight {
            self.weight
        }
    }

    struct MockBacklogStore {
        inner: HashMap<MockOrderId, BacklogOrder<MockOrder>>,
    }

    impl MockBacklogStore {
        fn new() -> Self {
            Self {
                inner: HashMap::new(),
            }
        }
    }

    impl UniqueOrder for MockOrder {
        type TOrderId = MockOrderId;

        fn get_self_ref(&self) -> Self::TOrderId {
            self.order_id
        }
    }

    #[async_trait]
    impl BacklogStore<MockOrder> for Arc<Mutex<MockBacklogStore>> {
        async fn put(&self, ord: BacklogOrder<MockOrder>) {
            self.lock().await.inner.insert(ord.order.order_id, ord);
        }

        async fn exists(&self, ord_id: MockOrderId) -> bool {
            self.lock().await.inner.contains_key(&ord_id)
        }

        async fn remove(&self, ord_id: MockOrderId) {
            self.lock().await.inner.remove(&ord_id);
        }

        async fn get(&self, ord_id: MockOrderId) -> Option<BacklogOrder<MockOrder>> {
            self.lock().await.inner.get(&ord_id).cloned()
        }

        async fn find_orders<F>(&self, f: F) -> Vec<BacklogOrder<MockOrder>>
        where
            F: Fn(&MockOrder) -> bool + Send + 'static,
        {
            self.lock()
                .await
                .inner
                .values()
                .cloned()
                .filter(|b| f(&b.order))
                .collect()
        }
    }

    async fn setup_backlog(
        order_lifespan_secs: i64,
        order_exec_time_secs: i64,
        retry_suspended_prob: u8,
    ) -> PersistentPriorityBacklog<MockOrder, Arc<Mutex<MockBacklogStore>>> {
        let store = Arc::new(Mutex::new(MockBacklogStore::new()));
        let conf = BacklogConfig {
            order_lifespan: Duration::seconds(order_lifespan_secs),
            order_exec_time: Duration::seconds(order_exec_time_secs),
            retry_suspended_prob: <BoundedU8<0, 100>>::new(retry_suspended_prob).unwrap(),
        };
        PersistentPriorityBacklog::new::<MockOrder>(store, conf).await
    }

    fn make_order(id: i64, weight: u64) -> BacklogOrder<MockOrder> {
        BacklogOrder {
            order: MockOrder {
                order_id: MockOrderId(id),
                weight: OrderWeight::from(weight),
            },
            timestamp: Utc::now().timestamp(),
        }
    }

    #[tokio::test]
    async fn should_suspend_existing_order() {
        let mut backlog = setup_backlog(10, 5, 50).await;
        let ord = make_order(1, 1);
        backlog.put(ord.clone().into()).await;
        let suspended = backlog.suspend(ord.order).await;
        assert!(suspended)
    }

    #[tokio::test]
    async fn should_check_later_existing_order() {
        let mut backlog = setup_backlog(10, 5, 50).await;
        let ord = make_order(1, 1);
        backlog.put(ord.clone().into()).await;
        let accepted = backlog.check_later(ord.into()).await;
        assert!(accepted)
    }

    #[tokio::test]
    async fn should_not_suspend_non_existent_order() {
        let mut backlog = setup_backlog(10, 5, 50).await;
        let ord = make_order(1, 1);
        let suspended = backlog.suspend(ord.order).await;
        assert!(!suspended)
    }

    #[tokio::test]
    async fn should_not_check_later_non_existent_order() {
        let mut backlog = setup_backlog(10, 5, 50).await;
        let ord = make_order(1, 1);
        let accepted = backlog
            .check_later(ProgressingOrder {
                order: ord.order,
                timestamp: ord.timestamp,
            })
            .await;
        assert!(!accepted)
    }

    #[tokio::test]
    async fn should_pop_best_order() {
        let mut backlog = setup_backlog(10, 5, 0).await;
        let ord1 = make_order(1, 1);
        let ord2 = make_order(2, 2);
        let ord3 = make_order(3, 3);
        backlog.put(ord1.into()).await;
        backlog.put(ord2.into()).await;
        backlog.put(ord3.clone().into()).await;

        let res = backlog.try_pop().await;
        assert_eq!(res, Some(ord3.order))
    }

    #[tokio::test]
    async fn should_always_pop_suspended_order_when_pa_100() {
        let mut backlog = setup_backlog(10, 5, 100).await;
        let ord1 = make_order(1, 1);
        let ord2 = make_order(2, 2);
        let ord3 = make_order(3, 3);
        backlog.put(ord1.into()).await;
        backlog.put(ord2.into()).await;
        backlog.put(ord3.clone().into()).await;
        let _ = backlog.try_pop().await;
        backlog.suspend(ord3.clone().order).await;

        let res = backlog.try_pop().await;
        assert_eq!(res, Some(ord3.order))
    }

    #[tokio::test]
    async fn should_not_pop_suspended_order_when_pa_0() {
        let mut backlog = setup_backlog(10, 5, 0).await;
        let ord1 = make_order(1, 1);
        let ord2 = make_order(2, 2);
        let ord3 = make_order(3, 3);
        backlog.put(ord1.into()).await;
        backlog.put(ord2.clone().into()).await;
        backlog.put(ord3.clone().into()).await;
        let _ = backlog.try_pop().await;
        backlog.suspend(ord3.clone().order).await;

        let res = backlog.try_pop().await;
        assert_eq!(res, Some(ord2.order))
    }

    #[tokio::test]
    async fn test_rocksdb_backlog() {
        let rnd = rand::thread_rng().next_u32();
        let mut store = BacklogStoreRocksDB {
            db: Arc::new(rocksdb::OptimisticTransactionDB::open_default(format!("./tmp/{}", rnd)).unwrap()),
        };
        for i in 0..30 {
            store.put(make_order(i, i as u64)).await;
        }

        // Github CI can be a bit slow, so timestamps don't coincide. Instead of equality we'll
        // check that they are within 2 seconds of each other.
        let check_eq = |ord1: BacklogOrder<MockOrder>, ord2: BacklogOrder<MockOrder>| {
            assert_eq!(ord1.order, ord2.order);
            assert!(ord1.timestamp.abs_diff(ord2.timestamp) < 2)
        };

        for i in 0..30 {
            assert!(<BacklogStoreRocksDB as BacklogStore<MockOrder>>::exists(&store, MockOrderId(i)).await);
            check_eq(
                make_order(i, i as u64),
                <BacklogStoreRocksDB as BacklogStore<MockOrder>>::get(&store, MockOrderId(i))
                    .await
                    .unwrap(),
            );
        }

        for i in 0..30 {
            <BacklogStoreRocksDB as BacklogStore<MockOrder>>::remove(&mut store, MockOrderId(i)).await;
            assert!(!<BacklogStoreRocksDB as BacklogStore<MockOrder>>::exists(&store, MockOrderId(i)).await);
            assert!(
                <BacklogStoreRocksDB as BacklogStore<MockOrder>>::get(&store, MockOrderId(i))
                    .await
                    .is_none()
            )
        }
    }
}
