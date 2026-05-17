use axum::{
    body::Body,
    extract::{Path, State},
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use std::path::Path as StdPath;

use crate::auth;
use crate::db;
use crate::routes::AppState;

pub async fn binary_handler(
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

    // 2. Fetch artifact record from database
    let artifact_info = {
        let conn = state.conn.lock().unwrap();
        db::get_artifact(&conn, &job_id)
    };

    let (file_path, _file_size, sha256_hash) = match artifact_info {
        Ok(Some(info)) => info,
        Ok(None) => return (StatusCode::NOT_FOUND, "Artifact not found for this build job").into_response(),
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, format!("Database query error: {}", e)).into_response(),
    };

    // 3. Verify file exists on disk
    let path = StdPath::new(&file_path);
    if !path.exists() {
        return (StatusCode::NOT_FOUND, "Artifact archive file missing from storage").into_response();
    }

    // 4. Read file bytes asynchronously
    let bytes = match tokio::fs::read(path).await {
        Ok(b) => b,
        Err(err) => return (StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to read archive from disk: {}", err)).into_response(),
    };

    // 5. Construct headers and response
    let response_headers = [
        (header::CONTENT_TYPE, "application/octet-stream"),
        (
            header::CONTENT_DISPOSITION,
            &format!("attachment; filename=\"{}.tar.gz\"", job_id),
        ),
        (
            header::HeaderName::from_static("x-sha256"),
            &sha256_hash,
        ),
    ];

    let mut response = Response::new(Body::from(bytes));
    *response.status_mut() = StatusCode::OK;
    
    for (k, v) in response_headers {
        if let Ok(val) = header::HeaderValue::from_str(v) {
            response.headers_mut().insert(k, val);
        }
    }

    response
}
