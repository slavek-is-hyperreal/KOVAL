use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};

use crate::auth;
use crate::db;
use crate::routes::AppState;

pub async fn status_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(job_id): Path<String>,
) -> Response {
    // 1. Authenticate and enforce rate-limiting
    let auth_header = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|h| h.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "));

    let token_str = match auth_header {
        Some(t) => t,
        None => return (StatusCode::UNAUTHORIZED, "Missing or invalid authorization header").into_response(),
    };

    let now = chrono::Utc::now();
    
    // Scoped lock for DB operations
    {
        let conn = state.conn.lock().unwrap();
        match auth::authenticate_and_rate_limit(&conn, token_str, now, state.rate_limit_limit) {
            Ok(_) => {}
            Err(auth::AuthError::InvalidToken) => return (StatusCode::UNAUTHORIZED, "Invalid token").into_response(),
            Err(auth::AuthError::InactiveToken) => return (StatusCode::UNAUTHORIZED, "Token is inactive").into_response(),
            Err(auth::AuthError::RateLimitExceeded) => return (StatusCode::TOO_MANY_REQUESTS, "Rate limit exceeded").into_response(),
            Err(auth::AuthError::DatabaseError(e)) => return (StatusCode::INTERNAL_SERVER_ERROR, format!("Database error: {}", e)).into_response(),
        }
    }

    // 2. Query job status from database
    let status_res = {
        let conn = state.conn.lock().unwrap();
        db::get_job_status(&conn, &job_id)
    };

    match status_res {
        Ok(Some(status)) => {
            let mut val = serde_json::to_value(&status).unwrap_or(serde_json::Value::Null);
            if status.status == "done" {
                if let Ok(Some((_, _, sha256))) = {
                    let conn = state.conn.lock().unwrap();
                    db::get_artifact(&conn, &job_id)
                } {
                    if let serde_json::Value::Object(ref mut map) = val {
                        map.insert("artifact_sha256".to_string(), serde_json::Value::String(sha256));
                    }
                }
            }
            (StatusCode::OK, Json(val)).into_response()
        }
        Ok(None) => (StatusCode::NOT_FOUND, "Job not found").into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("Database query error: {}", e)).into_response(),
    }
}
