use std::collections::HashMap;
use std::fmt::Debug;

use async_trait::async_trait;
use log::trace;

use crate::box_resolver::{Predicted, Traced};
use crate::data::unique_entity::{Confirmed, Unconfirmed};
use crate::data::OnChainEntity;

/// Stores on-chain entities.
/// Operations are atomic.
#[async_trait(? Send)]
pub trait EntityRepo<TEntity: OnChainEntity> {
    /// Get state id preceding given predicted state.
    async fn get_prediction_predecessor<'a>(&self, id: TEntity::TStateId) -> Option<TEntity::TStateId>
    where
        <TEntity as OnChainEntity>::TStateId: 'a;
    /// Get last predicted state of the given entity.
    async fn get_last_predicted<'a>(&self, id: TEntity::TEntityId) -> Option<Predicted<TEntity>>
    where
        <TEntity as OnChainEntity>::TEntityId: 'a;
    /// Get last confirmed state of the given entity.
    async fn get_last_confirmed<'a>(&self, id: TEntity::TEntityId) -> Option<Confirmed<TEntity>>
    where
        <TEntity as OnChainEntity>::TEntityId: 'a;
    /// Get last unconfirmed state of the given entity.
    async fn get_last_unconfirmed<'a>(&self, id: TEntity::TEntityId) -> Option<Unconfirmed<TEntity>>
    where
        <TEntity as OnChainEntity>::TEntityId: 'a;
    /// Persist predicted state of the entity.
    async fn put_predicted<'a>(&mut self, entity: Traced<Predicted<TEntity>>)
    where
        Traced<Predicted<TEntity>>: 'a;
    /// Persist confirmed state of the entity.
    async fn put_confirmed<'a>(&mut self, entity: Confirmed<TEntity>)
    where
        Traced<Predicted<TEntity>>: 'a;
    /// Persist unconfirmed state of the entity.
    async fn put_unconfirmed<'a>(&mut self, entity: Unconfirmed<TEntity>)
    where
        Traced<Predicted<TEntity>>: 'a;
    /// Invalidate particular state of the entity.
    async fn invalidate<'a>(&mut self, sid: TEntity::TStateId, eid: TEntity::TEntityId)
    where
        <TEntity as OnChainEntity>::TStateId: 'a,
        <TEntity as OnChainEntity>::TEntityId: 'a;
    /// Invalidate particular state of the entity.
    async fn eliminate<'a>(&mut self, entity: TEntity)
    where
        TEntity: 'a;
    /// False-positive analog of `exists()`.
    async fn may_exist<'a>(&self, sid: TEntity::TStateId) -> bool
    where
        <TEntity as OnChainEntity>::TStateId: 'a;
    async fn get_state<'a>(&self, sid: TEntity::TStateId) -> Option<TEntity>
    where
        <TEntity as OnChainEntity>::TStateId: 'a;
}

pub struct EntityRepoTracing<R> {
    inner: R,
}

impl<R> EntityRepoTracing<R> {
    pub fn wrap(repo: R) -> Self {
        Self { inner: repo }
    }
}

#[async_trait(? Send)]
impl<TEntity, R> EntityRepo<TEntity> for EntityRepoTracing<R>
where
    TEntity: OnChainEntity,
    TEntity::TEntityId: Debug + Copy,
    TEntity::TStateId: Debug + Copy,
    R: EntityRepo<TEntity>,
{
    async fn get_prediction_predecessor<'a>(&self, id: TEntity::TStateId) -> Option<TEntity::TStateId>
    where
        <TEntity as OnChainEntity>::TStateId: 'a,
    {
        trace!(target: "box_resolver", "get_prediction_predecessor({:?})", id);
        let res = self.inner.get_prediction_predecessor(id).await;
        trace!(target: "box_resolver", "get_prediction_predecessor({:?}) -> {:?}", id, res);
        res
    }

    async fn get_last_predicted<'a>(&self, id: TEntity::TEntityId) -> Option<Predicted<TEntity>>
    where
        <TEntity as OnChainEntity>::TEntityId: 'a,
    {
        trace!(target: "box_resolver", "get_last_predicted({:?})", id);
        let res = self.inner.get_last_predicted(id).await;
        trace!(target: "box_resolver", "get_last_predicted({:?}) -> {:?}", id, res.as_ref().map(|_| "<Entity>"));
        res
    }

    async fn get_last_confirmed<'a>(&self, id: TEntity::TEntityId) -> Option<Confirmed<TEntity>>
    where
        <TEntity as OnChainEntity>::TEntityId: 'a,
    {
        trace!(target: "box_resolver", "get_last_confirmed({:?})", id);
        let res = self.inner.get_last_confirmed(id).await;
        trace!(target: "box_resolver", "get_last_confirmed({:?}) -> {:?}", id, res.as_ref().map(|_| "<Entity>"));
        res
    }

    async fn get_last_unconfirmed<'a>(&self, id: TEntity::TEntityId) -> Option<Unconfirmed<TEntity>>
    where
        <TEntity as OnChainEntity>::TEntityId: 'a,
    {
        trace!(target: "box_resolver", "get_last_unconfirmed({:?})", id);
        let res = self.inner.get_last_unconfirmed(id).await;
        trace!(target: "box_resolver", "get_last_unconfirmed({:?}) -> {:?}", id, res.as_ref().map(|_| "<Entity>"));
        res
    }

    async fn put_predicted<'a>(&mut self, entity: Traced<Predicted<TEntity>>)
    where
        Traced<Predicted<TEntity>>: 'a,
    {
        let show_entity = format!(
            "<Entity({:?}, {:?})>",
            entity.state.get_self_ref(),
            entity.state.get_self_state_ref()
        );
        trace!(target: "box_resolver", "put_predicted({})", show_entity);
        self.inner.put_predicted(entity).await;
        trace!(target: "box_resolver", "put_predicted({}) -> ()", show_entity);
    }

    async fn put_confirmed<'a>(&mut self, entity: Confirmed<TEntity>)
    where
        Traced<Predicted<TEntity>>: 'a,
    {
        let show_entity = format!(
            "<Entity({:?}, {:?})>",
            entity.0.get_self_ref(),
            entity.0.get_self_state_ref()
        );
        trace!(target: "box_resolver", "put_confirmed({})", show_entity);
        self.inner.put_confirmed(entity).await;
        trace!(target: "box_resolver", "put_confirmed({}) -> ()", show_entity);
    }

    async fn put_unconfirmed<'a>(&mut self, entity: Unconfirmed<TEntity>)
    where
        Traced<Predicted<TEntity>>: 'a,
    {
        let show_entity = format!(
            "<Entity({:?}, {:?})>",
            entity.0.get_self_ref(),
            entity.0.get_self_state_ref()
        );
        trace!(target: "box_resolver", "put_unconfirmed({})", show_entity);
        self.inner.put_unconfirmed(entity).await;
        trace!(target: "box_resolver", "put_unconfirmed({}) -> ()", show_entity);
    }

    async fn invalidate<'a>(&mut self, sid: TEntity::TStateId, eid: TEntity::TEntityId)
    where
        <TEntity as OnChainEntity>::TStateId: 'a,
        <TEntity as OnChainEntity>::TEntityId: 'a,
    {
        trace!(target: "box_resolver", "invalidate({:?})", sid);
        self.inner.invalidate(sid, eid).await;
        trace!(target: "box_resolver", "invalidate({:?}) -> ()", sid);
    }

    async fn eliminate<'a>(&mut self, entity: TEntity)
    where
        TEntity: 'a,
    {
        let show_entity = format!(
            "<Entity({:?}, {:?})>",
            entity.get_self_ref(),
            entity.get_self_state_ref()
        );
        trace!(target: "box_resolver", "eliminate({})", show_entity);
        self.inner.eliminate(entity).await;
        trace!(target: "box_resolver", "eliminate({}) -> ()", show_entity);
    }

    async fn may_exist<'a>(&self, sid: TEntity::TStateId) -> bool
    where
        <TEntity as OnChainEntity>::TStateId: 'a,
    {
        self.inner.may_exist(sid).await
    }

    async fn get_state<'a>(&self, sid: TEntity::TStateId) -> Option<TEntity>
    where
        <TEntity as OnChainEntity>::TStateId: 'a,
    {
        trace!(target: "box_resolver", "get_state({:?})", sid);
        let res = self.inner.get_state(sid).await;
        let show_entity = res
            .as_ref()
            .map(|e| format!("<Entity({:?}, {:?})>", e.get_self_ref(), e.get_self_state_ref()));
        trace!(target: "box_resolver", "get_state({:?}) -> {:?}", sid, show_entity);
        res
    }
}

type InMemoryKey = [u8; 33];

pub struct EphemeralEntityRepo {
    store: HashMap<InMemoryKey, Vec<u8>>,
}

impl EphemeralEntityRepo {
    pub fn new() -> Self {
        Self {
            store: HashMap::new(),
        }
    }
}

#[async_trait(?Send)]
impl<TEntity> EntityRepo<TEntity> for EphemeralEntityRepo
where
    TEntity: OnChainEntity + Clone + Send + 'static,
    <TEntity as OnChainEntity>::TStateId: Clone + Send + 'static,
    <TEntity as OnChainEntity>::TEntityId: Clone + Send + 'static,
{
    async fn get_prediction_predecessor<'a>(&self, id: TEntity::TStateId) -> Option<TEntity::TStateId>
    where
        <TEntity as OnChainEntity>::TStateId: 'a,
    {
        todo!()
    }

    async fn get_last_predicted<'a>(&self, id: TEntity::TEntityId) -> Option<Predicted<TEntity>>
    where
        <TEntity as OnChainEntity>::TEntityId: 'a,
    {
        todo!()
    }

    async fn get_last_confirmed<'a>(&self, id: TEntity::TEntityId) -> Option<Confirmed<TEntity>>
    where
        <TEntity as OnChainEntity>::TEntityId: 'a,
    {
        todo!()
    }

    async fn get_last_unconfirmed<'a>(&self, id: TEntity::TEntityId) -> Option<Unconfirmed<TEntity>>
    where
        <TEntity as OnChainEntity>::TEntityId: 'a,
    {
        todo!()
    }

    async fn put_predicted<'a>(&mut self, entity: Traced<Predicted<TEntity>>)
    where
        Traced<Predicted<TEntity>>: 'a,
    {
        todo!()
    }

    async fn put_confirmed<'a>(&mut self, entity: Confirmed<TEntity>)
    where
        Traced<Predicted<TEntity>>: 'a,
    {
        todo!()
    }

    async fn put_unconfirmed<'a>(&mut self, entity: Unconfirmed<TEntity>)
    where
        Traced<Predicted<TEntity>>: 'a,
    {
        todo!()
    }

    async fn invalidate<'a>(&mut self, sid: TEntity::TStateId, eid: TEntity::TEntityId)
    where
        <TEntity as OnChainEntity>::TStateId: 'a,
        <TEntity as OnChainEntity>::TEntityId: 'a,
    {
        todo!()
    }

    async fn eliminate<'a>(&mut self, entity: TEntity)
    where
        TEntity: 'a,
    {
        todo!()
    }

    async fn may_exist<'a>(&self, sid: TEntity::TStateId) -> bool
    where
        <TEntity as OnChainEntity>::TStateId: 'a,
    {
        todo!()
    }

    async fn get_state<'a>(&self, sid: TEntity::TStateId) -> Option<TEntity>
    where
        <TEntity as OnChainEntity>::TStateId: 'a,
    {
        todo!()
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use std::sync::Arc;

    use rand::{thread_rng, RngCore};
    use serde::{Deserialize, Serialize};

    use crate::box_resolver::rocksdb::EntityRepoRocksDB;
    use crate::{
        box_resolver::persistence::EntityRepo,
        data::{
            unique_entity::{Confirmed, Predicted, Traced, Unconfirmed},
            OnChainEntity,
        },
    };

    #[repr(transparent)]
    #[derive(Serialize, Deserialize, Copy, Clone, Eq, PartialEq, Hash, Debug)]
    pub struct TokenId(u64);

    impl TokenId {
        pub fn random() -> Self {
            Self(thread_rng().next_u64())
        }
    }

    #[repr(transparent)]
    #[derive(Serialize, Deserialize, Copy, Clone, Eq, PartialEq, Hash, Debug)]
    pub struct BoxId(u64);

    impl BoxId {
        pub fn random() -> Self {
            Self(thread_rng().next_u64())
        }
    }

    #[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug)]
    pub struct TestEntity {
        pub token_id: TokenId,
        pub box_id: BoxId,
    }

    impl OnChainEntity for TestEntity {
        type TEntityId = TokenId;

        type TStateId = BoxId;

        fn get_self_ref(&self) -> Self::TEntityId {
            self.token_id
        }

        fn get_self_state_ref(&self) -> Self::TStateId {
            self.box_id
        }
    }

    #[tokio::test]
    async fn test_rocksdb_may_exist() {
        let client = rocks_db_client();
        test_entity_repo_may_exist(client).await;
    }

    #[tokio::test]
    async fn test_rocksdb_predicted() {
        let client = rocks_db_client();
        test_entity_repo_predicted(client).await;
    }

    #[tokio::test]
    async fn test_rocksdb_confirmed() {
        let client = rocks_db_client();
        test_entity_repo_confirmed(client).await;
    }

    #[tokio::test]
    async fn test_rocksdb_unconfirmed() {
        let client = rocks_db_client();
        test_entity_repo_unconfirmed(client).await;
    }

    #[tokio::test]
    async fn test_rocksdb_invalidate() {
        let client = rocks_db_client();
        test_entity_repo_invalidate(client).await;
    }

    #[tokio::test]
    async fn test_rocksdb_eliminate() {
        let client = rocks_db_client();
        test_entity_repo_eliminate(client).await;
    }

    pub fn rocks_db_client() -> EntityRepoRocksDB {
        let rnd = rand::thread_rng().next_u32();
        EntityRepoRocksDB {
            db: Arc::new(rocksdb::OptimisticTransactionDB::open_default(format!("./tmp/{}", rnd)).unwrap()),
        }
    }

    async fn test_entity_repo_may_exist<C: EntityRepo<TestEntity>>(mut client: C) {
        let (box_ids, token_ids, n) = gen_box_and_token_ids();
        for i in 1..n {
            let entity = Traced {
                state: Predicted(TestEntity {
                    token_id: token_ids[i],
                    box_id: box_ids[i],
                }),
                prev_state_id: box_ids.get(i - 1).cloned(),
            };
            client.put_predicted(entity.clone()).await;
        }
        #[allow(clippy::needless_range_loop)]
        for i in 1..n {
            let may_exist = client.may_exist(box_ids[i]).await;
            assert!(may_exist);
        }
    }

    async fn test_entity_repo_predicted<C: EntityRepo<TestEntity>>(mut client: C) {
        let (box_ids, token_ids, n) = gen_box_and_token_ids();
        for i in 1..n {
            let entity = Traced {
                state: Predicted(TestEntity {
                    token_id: token_ids[i],
                    box_id: box_ids[i],
                }),
                prev_state_id: box_ids.get(i - 1).cloned(),
            };
            client.put_predicted(entity.clone()).await;
        }
        for i in 1..n {
            let pred: Option<BoxId> = client.get_prediction_predecessor(box_ids[i]).await;
            assert_eq!(pred, box_ids.get(i - 1).cloned());
        }
    }

    async fn test_entity_repo_confirmed<C: EntityRepo<TestEntity>>(mut client: C) {
        let (box_ids, token_ids, n) = gen_box_and_token_ids();
        let mut entities = vec![];
        for i in 0..n {
            let entity = Confirmed(TestEntity {
                token_id: token_ids[i],
                box_id: box_ids[i],
            });
            client.put_confirmed(entity.clone()).await;
            entities.push(entity);
        }
        for i in 0..n {
            let e: Confirmed<TestEntity> = client.get_last_confirmed(token_ids[i]).await.unwrap();
            assert_eq!(e.0, entities[i].0);
        }
    }

    async fn test_entity_repo_unconfirmed<C: EntityRepo<TestEntity>>(mut client: C) {
        let (box_ids, token_ids, n) = gen_box_and_token_ids();
        let mut entities = vec![];
        for i in 0..n {
            let entity = Unconfirmed(TestEntity {
                token_id: token_ids[i],
                box_id: box_ids[i],
            });
            client.put_unconfirmed(entity.clone()).await;
            entities.push(entity);
        }
        for i in 0..n {
            let e: Unconfirmed<TestEntity> = client.get_last_unconfirmed(token_ids[i]).await.unwrap();
            assert_eq!(e.0, entities[i].0);
        }
    }

    async fn test_entity_repo_invalidate<C: EntityRepo<TestEntity>>(mut client: C) {
        let (box_ids, token_ids, n) = gen_box_and_token_ids();
        for i in 1..n {
            let ee = TestEntity {
                token_id: token_ids[i],
                box_id: box_ids[i],
            };
            let entity = Traced {
                state: Predicted(ee.clone()),
                prev_state_id: box_ids.get(i - 1).cloned(),
            };
            client.put_predicted(entity.clone()).await;
            client.put_unconfirmed(Unconfirmed(ee.clone())).await;
            client.put_confirmed(Confirmed(ee)).await;

            // Invalidate
            <C as EntityRepo<TestEntity>>::invalidate(&mut client, box_ids[i], token_ids[i]).await;
            let predicted: Option<Predicted<TestEntity>> = client.get_last_predicted(token_ids[i]).await;
            let unconfirmed: Option<Unconfirmed<TestEntity>> =
                client.get_last_unconfirmed(token_ids[i]).await;
            let confirmed: Option<Confirmed<TestEntity>> = client.get_last_confirmed(token_ids[i]).await;
            assert!(predicted.is_none());
            assert!(unconfirmed.is_none());
            if i == 1 {
                //  On first iteration of the loop, there is no confirmed entity to rollback to.
                assert!(confirmed.is_none());
            } else {
                // Here we rollback to confirmed entity.
                assert!(confirmed.is_some());
            }
        }
    }

    async fn test_entity_repo_eliminate<C: EntityRepo<TestEntity>>(mut client: C) {
        let (box_ids, token_ids, n) = gen_box_and_token_ids();
        for i in 1..n {
            let ee = TestEntity {
                token_id: token_ids[i],
                box_id: box_ids[i],
            };
            let entity = Traced {
                state: Predicted(ee.clone()),
                prev_state_id: box_ids.get(i - 1).cloned(),
            };
            client.put_predicted(entity.clone()).await;
            client.put_unconfirmed(Unconfirmed(ee.clone())).await;

            // Invalidate
            <C as EntityRepo<TestEntity>>::eliminate(&mut client, ee).await;
            let predicted: Option<Predicted<TestEntity>> = client.get_last_predicted(token_ids[i]).await;
            let unconfirmed: Option<Unconfirmed<TestEntity>> =
                client.get_last_unconfirmed(token_ids[i]).await;
            assert!(predicted.is_none());
            assert!(unconfirmed.is_none());
        }
    }

    fn gen_box_and_token_ids() -> (Vec<BoxId>, Vec<TokenId>, usize) {
        let box_ids: Vec<_> = (0..30).into_iter().map(|_| BoxId::random()).collect();
        let token_ids: Vec<_> = (0..30).into_iter().map(|_| TokenId::random()).collect();
        (box_ids, token_ids, 30)
    }
}
