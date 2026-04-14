use std::sync::Arc;

use log::trace;
use tokio::sync::Mutex;

use crate::box_resolver::persistence::EntityRepo;
use crate::domain::event::{Confirmed, Predicted, Traced, Unconfirmed};
use crate::domain::EntitySnapshot;

pub mod blacklist;
pub mod persistence;
pub mod process;

/// Get latest state of an on-chain entity `TEntity`.
pub async fn resolve_entity_state<TEntity, TRepo>(
    id: TEntity::StableId,
    repo: Arc<Mutex<TRepo>>,
) -> Option<TEntity>
where
    TRepo: EntityRepo<TEntity>,
    TEntity: EntitySnapshot,
    TEntity::StableId: Copy,
{
    let states = {
        let repo_guard = repo.lock().await;
        let confirmed = repo_guard.get_last_confirmed(id).await;
        let unconfirmed = repo_guard.get_last_unconfirmed(id).await;
        let predicted = repo_guard.get_last_predicted(id).await;
        (confirmed, unconfirmed, predicted)
    };
    let selected_confirmed_version = states.0.as_ref().map(|Confirmed(entity)| entity.version());
    let selected_unconfirmed_version = states.1.as_ref().map(|Unconfirmed(entity)| entity.version());
    let selected_predicted_version = states.2.as_ref().map(|Predicted(entity)| entity.version());
    trace!(
        target: "box_resolver",
        "resolve_entity_state: entity_type={} stable_id={} confirmed={} unconfirmed={} predicted={}",
        std::any::type_name::<TEntity>(),
        id,
        selected_confirmed_version
            .map(|version| version.to_string())
            .unwrap_or_else(|| "none".to_owned()),
        selected_unconfirmed_version
            .map(|version| version.to_string())
            .unwrap_or_else(|| "none".to_owned()),
        selected_predicted_version
            .map(|version| version.to_string())
            .unwrap_or_else(|| "none".to_owned()),
    );
    match states {
        (Some(Confirmed(conf)), unconf, Some(Predicted(pred))) => {
            let anchoring_point = if conf.is_quasi_permanent() {
                conf
            } else {
                unconf.map(|Unconfirmed(e)| e).unwrap_or(conf)
            };
            let anchoring_sid = anchoring_point.version();
            let predicted_sid = pred.version();
            let prediction_is_anchoring_point = predicted_sid == anchoring_sid;
            let prediction_is_valid = prediction_is_anchoring_point
                || is_linking(predicted_sid, anchoring_sid, Arc::clone(&repo)).await;
            let safe_point = if prediction_is_valid {
                pred
            } else {
                anchoring_point
            };
            trace!(
                target: "box_resolver",
                "resolve_entity_state: entity_type={} stable_id={} selected={}",
                std::any::type_name::<TEntity>(),
                id,
                safe_point.version(),
            );
            Some(safe_point)
        }
        (Some(Confirmed(conf)), Some(Unconfirmed(_)), None) if conf.is_quasi_permanent() => {
            trace!(
                target: "box_resolver",
                "resolve_entity_state: entity_type={} stable_id={} selected={}",
                std::any::type_name::<TEntity>(),
                id,
                conf.version(),
            );
            Some(conf)
        }
        (_, Some(Unconfirmed(unconf)), None) => {
            trace!(
                target: "box_resolver",
                "resolve_entity_state: entity_type={} stable_id={} selected={}",
                std::any::type_name::<TEntity>(),
                id,
                unconf.version(),
            );
            Some(unconf)
        }
        (Some(Confirmed(conf)), _, _) => {
            trace!(
                target: "box_resolver",
                "resolve_entity_state: entity_type={} stable_id={} selected={}",
                std::any::type_name::<TEntity>(),
                id,
                conf.version(),
            );
            Some(conf)
        }
        _ => {
            trace!(
                target: "box_resolver",
                "resolve_entity_state: entity_type={} stable_id={} selected=none",
                std::any::type_name::<TEntity>(),
                id,
            );
            None
        }
    }
}

async fn is_linking<TEntity, TRepo>(
    sid: TEntity::Version,
    anchoring_sid: TEntity::Version,
    repo: Arc<Mutex<TRepo>>,
) -> bool
where
    TEntity: EntitySnapshot,
    TRepo: EntityRepo<TEntity>,
{
    let mut head_sid = sid;
    let repo = repo.lock().await;
    loop {
        match repo.get_prediction_predecessor(head_sid).await {
            None => return false,
            Some(prev_state_id) if prev_state_id == anchoring_sid => return true,
            Some(prev_state_id) => head_sid = prev_state_id,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use async_trait::async_trait;
    use tokio::sync::Mutex;

    use crate::box_resolver::persistence::tests::*;
    use crate::box_resolver::persistence::EntityRepo;
    use crate::box_resolver::{resolve_entity_state, Predicted, Traced};
    use crate::domain::event::{Confirmed, Unconfirmed};
    use crate::domain::EntitySnapshot;
    use crate::domain::Stable;

    #[tokio::test]
    async fn test_resolve_state_trivial() {
        let mut client = rocks_db_client();
        let entity = Confirmed(TestEntity {
            token_id: TokenId::random(),
            box_id: BoxId::random(),
        });
        client.put_confirmed(entity.clone()).await;

        let client = Arc::new(Mutex::new(client));
        let resolved = resolve_entity_state::<TestEntity, _>(entity.0.stable_id(), client).await;
        assert_eq!(resolved, Some(entity.0));
    }

    #[tokio::test]
    async fn test_resolve_state_prefers_confirmed_over_stale_unconfirmed_for_quasi_permanent_entity() {
        let token_id = TokenId::random();
        let stale_unconfirmed = TestEntity {
            token_id,
            box_id: BoxId::random(),
        };
        let confirmed = TestEntity {
            token_id,
            box_id: BoxId::random(),
        };

        let mut client = rocks_db_client();
        client.put_unconfirmed(Unconfirmed(stale_unconfirmed.clone())).await;
        client.put_confirmed(Confirmed(confirmed.clone())).await;

        let client = Arc::new(Mutex::new(client));
        let resolved = resolve_entity_state::<TestEntity, _>(token_id, client).await;
        assert_eq!(resolved, Some(confirmed));
    }

    #[tokio::test]
    async fn test_resolve_state_prefers_confirmed_when_stale_unconfirmed_coexists_in_repo() {
        let token_id = TokenId::random();
        let confirmed = TestEntity {
            token_id,
            box_id: BoxId::random(),
        };
        let stale_unconfirmed = TestEntity {
            token_id,
            box_id: BoxId::random(),
        };

        let repo = TestResolverRepo {
            confirmed: Some(confirmed.clone()),
            unconfirmed: Some(stale_unconfirmed),
            predicted: None,
            predecessor: None,
        };

        let resolved = resolve_entity_state::<TestEntity, _>(token_id, Arc::new(Mutex::new(repo))).await;
        assert_eq!(resolved, Some(confirmed));
    }

    #[tokio::test]
    async fn test_resolve_state_keeps_predicted_chain_anchored_to_confirmed_for_quasi_permanent_entity() {
        let token_id = TokenId::random();
        let confirmed = TestEntity {
            token_id,
            box_id: BoxId::random(),
        };
        let stale_unconfirmed = TestEntity {
            token_id,
            box_id: BoxId::random(),
        };
        let predicted = TestEntity {
            token_id,
            box_id: BoxId::random(),
        };

        let repo = TestResolverRepo {
            confirmed: Some(confirmed.clone()),
            unconfirmed: Some(stale_unconfirmed),
            predicted: Some(predicted.clone()),
            predecessor: Some(confirmed.box_id),
        };

        let resolved = resolve_entity_state::<TestEntity, _>(token_id, Arc::new(Mutex::new(repo))).await;
        assert_eq!(resolved, Some(predicted));
    }

    struct TestResolverRepo {
        confirmed: Option<TestEntity>,
        unconfirmed: Option<TestEntity>,
        predicted: Option<TestEntity>,
        predecessor: Option<BoxId>,
    }

    #[async_trait]
    impl EntityRepo<TestEntity> for TestResolverRepo {
        async fn get_prediction_predecessor<'a>(&self, _id: BoxId) -> Option<BoxId>
        where
            <TestEntity as EntitySnapshot>::Version: 'a,
        {
            self.predecessor
        }

        async fn get_last_predicted<'a>(&self, _id: TokenId) -> Option<Predicted<TestEntity>>
        where
            <TestEntity as Stable>::StableId: 'a,
        {
            self.predicted.clone().map(Predicted)
        }

        async fn get_last_confirmed<'a>(&self, _id: TokenId) -> Option<Confirmed<TestEntity>>
        where
            <TestEntity as Stable>::StableId: 'a,
        {
            self.confirmed.clone().map(Confirmed)
        }

        async fn get_last_unconfirmed<'a>(&self, _id: TokenId) -> Option<Unconfirmed<TestEntity>>
        where
            <TestEntity as Stable>::StableId: 'a,
        {
            self.unconfirmed.clone().map(Unconfirmed)
        }

        async fn put_predicted<'a>(&mut self, _entity: Traced<Predicted<TestEntity>>)
        where
            Traced<Predicted<TestEntity>>: 'a,
        {
            unimplemented!()
        }

        async fn put_confirmed<'a>(&mut self, _entity: Confirmed<TestEntity>)
        where
            Traced<Predicted<TestEntity>>: 'a,
        {
            unimplemented!()
        }

        async fn put_unconfirmed<'a>(&mut self, _entity: Unconfirmed<TestEntity>)
        where
            Traced<Predicted<TestEntity>>: 'a,
        {
            unimplemented!()
        }

        async fn invalidate<'a>(&mut self, _sid: BoxId, _eid: TokenId)
        where
            <TestEntity as EntitySnapshot>::Version: 'a,
            <TestEntity as Stable>::StableId: 'a,
        {
            unimplemented!()
        }

        async fn eliminate<'a>(&mut self, _entity: TestEntity)
        where
            TestEntity: 'a,
        {
            unimplemented!()
        }

        async fn may_exist<'a>(&self, _sid: BoxId) -> bool
        where
            <TestEntity as EntitySnapshot>::Version: 'a,
        {
            false
        }

        async fn get_state<'a>(&self, _sid: BoxId) -> Option<TestEntity>
        where
            <TestEntity as EntitySnapshot>::Version: 'a,
        {
            None
        }
    }
}
