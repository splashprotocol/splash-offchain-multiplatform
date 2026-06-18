use crate::metrics::{compute_batcher_metrics, MetricsFilter};
use crate::store::ExecutionIndex;
use actix_cors::Cors;
use actix_web::dev::Server;
use actix_web::web::Data;
use actix_web::{guard, web, App, HttpResponse, HttpServer, Responder};
use async_primitives::beacon::Beacon;
use std::io;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

#[derive(Clone, Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MetricsQuery {
    pub from_ms: Option<u64>,
    pub to_ms: Option<u64>,
    pub pair: Option<String>,
}

async fn healthcheck(state_synced: Data<Beacon>, startup_complete: Data<Arc<AtomicBool>>) -> impl Responder {
    if !startup_complete.load(Ordering::Acquire) || !state_synced.read() {
        return HttpResponse::ServiceUnavailable().finish();
    }
    HttpResponse::Ok().finish()
}

async fn batchers<Index>(index: Data<Index>) -> impl Responder
where
    Index: ExecutionIndex + Send + Sync + 'static,
{
    let mut batchers = index.snapshot().await.batchers;
    batchers.sort_by(|left, right| left.pkh.cmp(&right.pkh));
    HttpResponse::Ok().json(batchers)
}

async fn batcher<Index>(path: web::Path<String>, index: Data<Index>) -> impl Responder
where
    Index: ExecutionIndex + Send + Sync + 'static,
{
    let pkh = path.into_inner();
    let snapshot = index.snapshot().await;
    match snapshot.batchers.into_iter().find(|batcher| batcher.pkh == pkh) {
        Some(batcher) => HttpResponse::Ok().json(batcher),
        None => HttpResponse::NotFound().finish(),
    }
}

async fn batcher_metrics<Index>(
    path: web::Path<String>,
    query: web::Query<MetricsQuery>,
    index: Data<Index>,
) -> impl Responder
where
    Index: ExecutionIndex + Send + Sync + 'static,
{
    let snapshot = index.snapshot().await;
    let metrics = compute_batcher_metrics(
        &snapshot,
        &path.into_inner(),
        MetricsFilter {
            from_ms: query.from_ms,
            to_ms: query.to_ms,
            pair: query.pair.clone(),
        },
    );
    HttpResponse::Ok().json(metrics)
}

pub async fn build_api_server<Index>(
    index: Index,
    state_synced: Beacon,
    startup_complete: Arc<AtomicBool>,
    bind_addr: SocketAddr,
) -> Result<Server, io::Error>
where
    Index: ExecutionIndex + Send + Sync + Clone + 'static,
{
    Ok(HttpServer::new(move || {
        let cors = Cors::default()
            .allow_any_origin()
            .allow_any_method()
            .allow_any_header();

        App::new()
            .wrap(cors)
            .app_data(Data::new(index.clone()))
            .app_data(Data::new(state_synced.clone()))
            .app_data(Data::new(startup_complete.clone()))
            .service(web::resource("/health").route(web::route().guard(guard::Get()).to(healthcheck)))
            .service(web::resource("/batchers").route(web::route().guard(guard::Get()).to(batchers::<Index>)))
            .service(
                web::resource("/batchers/{pkh}").route(web::route().guard(guard::Get()).to(batcher::<Index>)),
            )
            .service(
                web::resource("/batchers/{pkh}/metrics")
                    .route(web::route().guard(guard::Get()).to(batcher_metrics::<Index>)),
            )
    })
    .bind(bind_addr)?
    .workers(4)
    .disable_signals()
    .run())
}
