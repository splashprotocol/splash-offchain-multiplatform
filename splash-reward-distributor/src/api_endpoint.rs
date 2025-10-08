use axum::{extract::State, http::StatusCode, response::IntoResponse, Json};
use cml_chain::transaction::Transaction;
use futures::channel::mpsc;
use futures::SinkExt;
use tokio::sync::oneshot;

use crate::engine::verifier::TxCosignRequest;

#[derive(Clone)]
pub struct VerifierAppState {
    pub request_sender: mpsc::Sender<(TxCosignRequest, oneshot::Sender<Option<Transaction>>)>,
}

pub async fn handle_request_cosignature(
    State(state): State<VerifierAppState>,
    axum::Json(cosign_request): axum::Json<TxCosignRequest>,
) -> impl IntoResponse {
    let VerifierAppState { mut request_sender } = state;
    let (snd, recv) = tokio::sync::oneshot::channel();
    request_sender.send((cosign_request, snd)).await.unwrap();
    match recv.await {
        Ok(Some(signed_tx)) => (StatusCode::OK, Json(Some(signed_tx))),
        Ok(None) | Err(_) => (StatusCode::UNPROCESSABLE_ENTITY, Json(None)),
    }
}
