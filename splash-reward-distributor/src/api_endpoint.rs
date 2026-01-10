use axum::body::Body;
use axum::extract::Request;
use axum::{extract::State, http::StatusCode, response::IntoResponse, Json};
use cml_chain::transaction::Transaction;
use futures::channel::mpsc;
use futures::SinkExt;
use log::trace;
use tokio::sync::oneshot;

use crate::engine::verifier::TxCosignRequest;

#[derive(Clone)]
pub struct VerifierAppState {
    pub request_sender: mpsc::Sender<(TxCosignRequest, oneshot::Sender<Option<Transaction>>)>,
}

pub async fn handle_request_cosignature_bytes(
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

pub async fn handle_request_cosignature(
    State(state): State<VerifierAppState>,
    request: Request<Body>,
) -> impl IntoResponse {
    // Extract the body bytes
    trace!("handle_request_cosignature: {:?}", request);
    let body_bytes = match axum::body::to_bytes(request.into_body(), usize::MAX).await {
        Ok(bytes) => bytes,
        Err(e) => {
            return (StatusCode::BAD_REQUEST, Json(None));
        }
    };
    trace!("handle_request_cosignature: converted to bytes");

    trace!(
        "handle_request_cosignature: body bytes: {}",
        hex::encode(&body_bytes)
    );
    // Deserialize MessagePack
    let cosign_request: TxCosignRequest = match rmp_serde::from_slice(&body_bytes) {
        Ok(req) => req,
        Err(e) => {
            return (StatusCode::BAD_REQUEST, Json(None));
        }
    };
    trace!("handle_request_cosignature: converted to cosign request");

    let VerifierAppState { mut request_sender } = state;
    let (snd, recv) = tokio::sync::oneshot::channel();
    request_sender.send((cosign_request, snd)).await.unwrap();
    match recv.await {
        Ok(Some(signed_tx)) => {
            use cml_chain::Serialize;
            trace!("handle_request_cosignature: sent cosign request");
            (StatusCode::OK, Json(Some(hex::encode(signed_tx.to_cbor_bytes()))))
        }
        Ok(None) => {
            trace!("handle_request_cosignature: received none");
            (StatusCode::UNPROCESSABLE_ENTITY, Json(None))
        }
        Err(_) => {
            trace!("handle_request_cosignature: received error");
            (StatusCode::UNPROCESSABLE_ENTITY, Json(None))
        }
    }
}
