use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use schema::WebhookRequest;
use crate::auth;
use crate::db;
use crate::routes::AppState;

pub async fn register_webhook_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<WebhookRequest>,
) -> Response {
    let auth_header = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|h| h.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "));

    let token_str = match auth_header {
        Some(t) => t,
        None => return (StatusCode::UNAUTHORIZED, "Missing or invalid authorization header").into_response(),
    };

    let now = chrono::Utc::now();
    let token = {
        let conn = state.conn.lock().unwrap();
        match auth::authenticate_and_rate_limit(&conn, token_str, now, state.rate_limit_limit) {
            Ok(t) => t,
            Err(e) => return (StatusCode::UNAUTHORIZED, e.to_string()).into_response(),
        }
    };

    let created_at = now.to_rfc3339();
    let conn = state.conn.lock().unwrap();
    match db::insert_webhook(&conn, token.id, &payload.url, &payload.secret, &created_at) {
        Ok(id) => (StatusCode::CREATED, Json(serde_json::json!({ "id": id }))).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("Database error: {}", e)).into_response(),
    }
}

pub async fn list_webhooks_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Response {
    let auth_header = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|h| h.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "));

    let token_str = match auth_header {
        Some(t) => t,
        None => return (StatusCode::UNAUTHORIZED, "Missing or invalid authorization header").into_response(),
    };

    let now = chrono::Utc::now();
    let token = {
        let conn = state.conn.lock().unwrap();
        match auth::authenticate_and_rate_limit(&conn, token_str, now, state.rate_limit_limit) {
            Ok(t) => t,
            Err(e) => return (StatusCode::UNAUTHORIZED, e.to_string()).into_response(),
        }
    };

    let conn = state.conn.lock().unwrap();
    match db::get_webhooks_for_token(&conn, token.id) {
        Ok(list) => (StatusCode::OK, Json(list)).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("Database error: {}", e)).into_response(),
    }
}

pub async fn delete_webhook_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<i64>,
) -> Response {
    let auth_header = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|h| h.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "));

    let token_str = match auth_header {
        Some(t) => t,
        None => return (StatusCode::UNAUTHORIZED, "Missing or invalid authorization header").into_response(),
    };

    let now = chrono::Utc::now();
    let token = {
        let conn = state.conn.lock().unwrap();
        match auth::authenticate_and_rate_limit(&conn, token_str, now, state.rate_limit_limit) {
            Ok(t) => t,
            Err(e) => return (StatusCode::UNAUTHORIZED, e.to_string()).into_response(),
        }
    };

    let conn = state.conn.lock().unwrap();
    match db::deactivate_webhook(&conn, id, token.id) {
        Ok(_) => StatusCode::NO_CONTENT.into_response(),
        Err(rusqlite::Error::QueryReturnedNoRows) => (StatusCode::NOT_FOUND, "Webhook not found or does not belong to token").into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("Database error: {}", e)).into_response(),
    }
}
