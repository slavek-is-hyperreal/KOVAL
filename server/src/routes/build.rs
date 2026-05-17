use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use uuid::Uuid;
use schema::JobRequest;

use crate::auth;
use crate::db;
use crate::queue::{Job, QueueError};
use crate::routes::AppState;

pub async fn build_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<JobRequest>,
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
    let token = {
        let conn = state.conn.lock().unwrap();
        match auth::authenticate_and_rate_limit(&conn, token_str, now, state.rate_limit_limit) {
            Ok(t) => t,
            Err(auth::AuthError::InvalidToken) => return (StatusCode::UNAUTHORIZED, "Invalid token").into_response(),
            Err(auth::AuthError::InactiveToken) => return (StatusCode::UNAUTHORIZED, "Token is inactive").into_response(),
            Err(auth::AuthError::RateLimitExceeded) => return (StatusCode::TOO_MANY_REQUESTS, "Rate limit exceeded").into_response(),
            Err(auth::AuthError::DatabaseError(e)) => return (StatusCode::INTERNAL_SERVER_ERROR, format!("Database error: {}", e)).into_response(),
        }
    };

    // 2. Gather hardware profile using target probe binary (from target device payload)
    let hardware = payload.hardware.clone();

    // 3. Create job entity
    let job_id = Uuid::new_v4().to_string();
    let job = Job {
        id: job_id.clone(),
        token_id: token.id,
        project: payload.project.clone(),
        git_ref: payload.git_ref.clone(),
        hardware,
        binary: payload.binary.clone(),
    };

    // 4. Save job state in database
    {
        let conn = state.conn.lock().unwrap();
        if let Err(e) = db::insert_job(
            &conn,
            &job.id,
            job.token_id,
            &job.project,
            &job.git_ref,
            &job.hardware,
            &now.to_rfc3339(),
        ) {
            return (StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to record job in database: {}", e)).into_response();
        }
    }

    // 5. Enqueue job (applies immediate backpressure if full)
    match state.queue.enqueue(job) {
        Ok(id) => {
            let response_body = serde_json::json!({ "id": id });
            (StatusCode::ACCEPTED, Json(response_body)).into_response()
        }
        Err(QueueError::QueueFull) => {
            // Revert job record in DB or mark failed
            {
                let conn = state.conn.lock().unwrap();
                let finished_now = chrono::Utc::now().to_rfc3339();
                db::update_job_status(
                    &conn,
                    &job_id,
                    "failed",
                    None,
                    Some(&finished_now),
                    Some("Build queue is full. Service temporarily unavailable."),
                ).ok();
            }
            (StatusCode::SERVICE_UNAVAILABLE, "Build queue is full. Try again later.").into_response()
        }
        Err(QueueError::SendError(e)) => {
            (StatusCode::INTERNAL_SERVER_ERROR, format!("Job dispatch failed: {}", e)).into_response()
        }
    }
}
