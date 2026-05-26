//! HTTP endpoints for health checks.
//! Provides GET /health for health checks.

use crate::health::{AgentNodeStatus, EngineStatus, GetHealth};
use axum::{extract::State, http::StatusCode, response::IntoResponse, routing::get, Json, Router};
use serde::Serialize;
use std::time::Duration;

/// Health monitor state wrapper for the API handler.
/// Holds the sender to request current health from the health monitor task.
/// UnboundedSender is Clone + Send + Sync, so handlers clone it to send without locking.
pub struct HealthMonitorState<EStatus, NStatus> {
    pub sender: futures::channel::mpsc::UnboundedSender<GetHealth<EStatus, NStatus>>,
}

impl<EStatus, NStatus> Clone for HealthMonitorState<EStatus, NStatus> {
    fn clone(&self) -> Self {
        Self {
            sender: self.sender.clone(),
        }
    }
}

/// GET /health - Health check endpoint.
/// Queries the health monitor and returns current health or an error with appropriate status code.
pub async fn get_health_handler<EStatus, NStatus>(
    State(health_state): State<HealthMonitorState<EStatus, NStatus>>,
) -> impl IntoResponse
where
    EStatus: Serialize + Send + 'static,
    NStatus: Serialize + Send + 'static,
{
    let (tx, rx) = futures::channel::oneshot::channel();
    let get_health = GetHealth(tx);

    if health_state.sender.unbounded_send(get_health).is_err() {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({
                "error": "Health monitor unavailable"
            })),
        )
            .into_response();
    }

    match tokio::time::timeout(Duration::from_secs(5), rx).await {
        Ok(Ok(health)) => (StatusCode::OK, Json(health)).into_response(),
        Ok(Err(_)) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({
                "error": "Health monitor channel closed"
            })),
        )
            .into_response(),
        Err(_) => (
            StatusCode::GATEWAY_TIMEOUT,
            Json(serde_json::json!({
                "error": "Health monitor timeout"
            })),
        )
            .into_response(),
    }
}

/// Creates the router with only the health endpoint (unprotected).
pub fn create_health_router(health_monitor: HealthMonitorState<EngineStatus, AgentNodeStatus>) -> Router {
    Router::new()
        .route(
            "/health",
            get(get_health_handler::<EngineStatus, AgentNodeStatus>),
        )
        .with_state(health_monitor)
}
