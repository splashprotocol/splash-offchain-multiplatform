use axum::{extract::State, http::StatusCode, response::IntoResponse, Json};
use splash_dao_offchain::{
    entities::onchain::voting_escrow::Owner,
    routines::{DaoBotCommand, DaoBotMessage, DaoBotResponse},
};

use crate::AppState;

pub async fn handle_get_mve_status(
    State(state): State<AppState>,
    axum::Json(owner): axum::Json<Owner>,
) -> impl IntoResponse {
    let AppState { sender } = state;
    let (response_sender, recv) = tokio::sync::oneshot::channel();
    let msg = DaoBotMessage {
        command: DaoBotCommand::GetMVEOrderStatus {
            mve_order_owner: owner,
        },
        response_sender,
    };
    sender.send(msg).await.unwrap();
    match recv.await {
        Ok(status) => match status {
            DaoBotResponse::MVEStatus(status) => (StatusCode::OK, Json(Some(status))),
            DaoBotResponse::VotingOrder(_) => (StatusCode::UNPROCESSABLE_ENTITY, Json(None)),
        },
        Err(_err) => (StatusCode::UNPROCESSABLE_ENTITY, Json(None)),
    }
}
