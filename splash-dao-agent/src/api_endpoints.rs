use axum::{extract::State, http::StatusCode, response::IntoResponse, Json};
use splash_dao_offchain::{
    entities::{
        offchain::{
            voting_order::VotingOrder, ExtendVotingEscrowOffChainOrder, RedeemVotingEscrowOffChainOrder,
        },
        onchain::voting_escrow::Owner,
    },
    routines::{DaoBotCommand, DaoBotMessage, DaoBotResponse, VotingOrderCommand, VotingOrderStatus},
};

use crate::AppState;

pub async fn handle_voting_put(
    State(state): State<AppState>,
    axum::Json(payload): axum::Json<VotingOrder>,
) -> impl IntoResponse {
    let AppState { sender } = state;
    let (response_sender, recv) = tokio::sync::oneshot::channel();
    let msg = DaoBotMessage {
        command: DaoBotCommand::VotingOrder(VotingOrderCommand::Submit(payload)),
        response_sender,
    };
    sender.send(msg).await.unwrap();
    match recv.await {
        Ok(response) => match response {
            DaoBotResponse::VotingOrder(voting_order_status) => match voting_order_status {
                VotingOrderStatus::Queued | VotingOrderStatus::Success => {
                    (StatusCode::OK, format!("{:?}", voting_order_status))
                }
                VotingOrderStatus::Failed => {
                    (StatusCode::UNPROCESSABLE_ENTITY, "TX submission failed".into())
                }
                VotingOrderStatus::VotingEscrowNotFound => (
                    StatusCode::NOT_FOUND,
                    "Cannot find associated voting_escrow".into(),
                ),
            },
            DaoBotResponse::MVEStatus(mve_status) => (
                StatusCode::UNPROCESSABLE_ENTITY,
                format!("Unexpected response: MVEStatus: {:?}", mve_status),
            ),
        },
        Err(_err) => (StatusCode::UNPROCESSABLE_ENTITY, "Unknown error".into()),
    }
}

pub async fn handle_extend_ve_put(
    State(state): State<AppState>,
    axum::Json(payload): axum::Json<ExtendVotingEscrowOffChainOrder>,
) -> impl IntoResponse {
    let AppState { sender } = state;
    let (response_sender, recv) = tokio::sync::oneshot::channel();
    let msg = DaoBotMessage {
        command: DaoBotCommand::ExtendVotingEscrowOrder(payload),
        response_sender,
    };
    sender.send(msg).await.unwrap();
    match recv.await {
        Ok(response) => match response {
            DaoBotResponse::VotingOrder(voting_order_status) => match voting_order_status {
                VotingOrderStatus::Queued | VotingOrderStatus::Success => {
                    (StatusCode::OK, format!("{:?}", voting_order_status))
                }
                VotingOrderStatus::Failed => {
                    (StatusCode::UNPROCESSABLE_ENTITY, "TX submission failed".into())
                }
                VotingOrderStatus::VotingEscrowNotFound => (
                    StatusCode::NOT_FOUND,
                    "Cannot find associated voting_escrow".into(),
                ),
            },
            DaoBotResponse::MVEStatus(mve_status) => (
                StatusCode::UNPROCESSABLE_ENTITY,
                format!("Unexpected response: MVEStatus: {:?}", mve_status),
            ),
        },
        Err(_err) => (StatusCode::UNPROCESSABLE_ENTITY, "Unknown error".into()),
    }
}

pub async fn handle_redeem_ve_put(
    State(state): State<AppState>,
    axum::Json(payload): axum::Json<RedeemVotingEscrowOffChainOrder>,
) -> impl IntoResponse {
    let AppState { sender } = state;
    let (response_sender, recv) = tokio::sync::oneshot::channel();
    let msg = DaoBotMessage {
        command: DaoBotCommand::RedeemVotingEscrowOrder(payload),
        response_sender,
    };
    sender.send(msg).await.unwrap();
    match recv.await {
        Ok(response) => match response {
            DaoBotResponse::VotingOrder(voting_order_status) => match voting_order_status {
                VotingOrderStatus::Queued | VotingOrderStatus::Success => {
                    (StatusCode::OK, format!("{:?}", voting_order_status))
                }
                VotingOrderStatus::Failed => {
                    (StatusCode::UNPROCESSABLE_ENTITY, "TX submission failed".into())
                }
                VotingOrderStatus::VotingEscrowNotFound => (
                    StatusCode::NOT_FOUND,
                    "Cannot find associated voting_escrow".into(),
                ),
            },
            DaoBotResponse::MVEStatus(mve_status) => (
                StatusCode::UNPROCESSABLE_ENTITY,
                format!("Unexpected response: MVEStatus: {:?}", mve_status),
            ),
        },
        Err(_err) => (StatusCode::UNPROCESSABLE_ENTITY, "Unknown error".into()),
    }
}

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
