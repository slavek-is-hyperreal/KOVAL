use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use uuid::Uuid;
use schema::{TokenRequest, TokenResponse};
use crate::auth;
use crate::db;
use crate::routes::AppState;

pub async fn create_token_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<TokenRequest>,
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
    
    // Authenticate admin token via standard bcrypt & rate-limit
    {
        let conn = state.conn.lock().unwrap();
        match auth::authenticate_and_rate_limit(&conn, token_str, now, state.rate_limit_limit) {
            Ok(_) => {}
            Err(e) => return (StatusCode::UNAUTHORIZED, e.to_string()).into_response(),
        }
    }

    if token_str != "koval_tkn_default_admin" {
        return (StatusCode::FORBIDDEN, "Access denied: Admin privileges required").into_response();
    }

    // Generate new plaintext token as UUID v4
    let plaintext_token = Uuid::new_v4().to_string();
    let hashed_token = match auth::hash_token(&plaintext_token) {
        Ok(h) => h,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to hash token: {:?}", e)).into_response(),
    };

    let created_at = now.to_rfc3339();
    let conn = state.conn.lock().unwrap();
    match db::insert_token(&conn, &hashed_token, &payload.name, &created_at) {
        Ok(id) => {
            let response = TokenResponse {
                id,
                plaintext_token,
                name: payload.name,
            };
            (StatusCode::CREATED, Json(response)).into_response()
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("Database error: {}", e)).into_response(),
    }
}

pub async fn list_tokens_handler(
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
    
    {
        let conn = state.conn.lock().unwrap();
        match auth::authenticate_and_rate_limit(&conn, token_str, now, state.rate_limit_limit) {
            Ok(_) => {}
            Err(e) => return (StatusCode::UNAUTHORIZED, e.to_string()).into_response(),
        }
    }

    if token_str != "koval_tkn_default_admin" {
        return (StatusCode::FORBIDDEN, "Access denied: Admin privileges required").into_response();
    }

    let conn = state.conn.lock().unwrap();
    match db::get_active_token_records(&conn) {
        Ok(list) => (StatusCode::OK, Json(list)).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("Database error: {}", e)).into_response(),
    }
}

pub async fn delete_token_handler(
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
    
    {
        let conn = state.conn.lock().unwrap();
        match auth::authenticate_and_rate_limit(&conn, token_str, now, state.rate_limit_limit) {
            Ok(_) => {}
            Err(e) => return (StatusCode::UNAUTHORIZED, e.to_string()).into_response(),
        }
    }

    if token_str != "koval_tkn_default_admin" {
        return (StatusCode::FORBIDDEN, "Access denied: Admin privileges required").into_response();
    }

    let conn = state.conn.lock().unwrap();
    match db::revoke_token(&conn, id) {
        Ok(_) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("Database error: {}", e)).into_response(),
    }
}
