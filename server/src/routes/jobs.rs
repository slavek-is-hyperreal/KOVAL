use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use crate::auth;
use crate::db;
use crate::routes::AppState;

/// Lists the last 50 jobs for the authenticated developer token.
pub async fn list_jobs_handler(
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
    match db::get_recent_jobs(&conn, token.id, 50) {
        Ok(list) => (StatusCode::OK, Json(list)).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("Database error: {}", e)).into_response(),
    }
}
