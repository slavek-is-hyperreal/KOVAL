use axum::{
    extract::{Path, Query, State, Json},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
};
use uuid::Uuid;
use serde::Deserialize;
use crate::routes::AppState;
use crate::auth;
use crate::db;
use crate::queue::{Job, QueueError};

#[derive(Deserialize)]
pub struct InstallParams {
    #[serde(rename = "ref")]
    pub git_ref: Option<String>,
    pub token: Option<String>,
}

#[derive(Deserialize)]
pub struct ForgeInstallParams {
    pub project: String,
    #[serde(rename = "ref")]
    pub git_ref: String,
}

pub async fn install_script_handler(
    Path(project): Path<String>,
    Query(params): Query<InstallParams>,
    headers: HeaderMap,
) -> impl IntoResponse {
    let host = headers
        .get(axum::http::header::HOST)
        .and_then(|h| h.to_str().ok())
        .unwrap_or("localhost:8090");
    
    let scheme = if host.starts_with("localhost") || host.starts_with("127.0.0.1") || host.starts_with("0.0.0.0") {
        "http"
    } else {
        "https"
    };
    let server_url = format!("{}://{}", scheme, host);
    let template = include_str!("../assets/install.sh");

    let git_ref = params.git_ref.unwrap_or_else(|| "main".to_string());
    let token = params.token.unwrap_or_default();

    let rendered = template
        .replace("{{SERVER_URL}}", &server_url)
        .replace("{{PROJECT}}", &project)
        .replace("{{REF}}", &git_ref)
        .replace("{{TOKEN}}", &token);

    (
        [
            (axum::http::header::CONTENT_TYPE, "text/x-shellscript"),
            (axum::http::header::CONTENT_DISPOSITION, "attachment; filename=\"install.sh\""),
        ],
        rendered,
    )
}

pub async fn static_probe_handler(
    Path(arch): Path<String>,
) -> impl IntoResponse {
    let bytes: &'static [u8] = match arch.as_str() {
        "x86_64" => include_bytes!("../assets/probe_x86_64"),
        "aarch64" => include_bytes!("../assets/probe_aarch64"),
        _ => return (StatusCode::NOT_FOUND, "Unsupported architecture").into_response(),
    };

    (
        [
            (axum::http::header::CONTENT_TYPE, "application/octet-stream"),
            (axum::http::header::CONTENT_DISPOSITION, &format!("attachment; filename=\"koval-probe-static-{}\"", arch)),
        ],
        bytes,
    ).into_response()
}

pub async fn forge_install_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(params): Query<ForgeInstallParams>,
    Json(hardware): Json<schema::HardwareProfile>,
) -> impl IntoResponse {
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

    // 2. Build Cache Lookup
    let hardware_json = serde_json::to_string(&hardware).unwrap_or_default();
    let cache_key = crate::cache::compute_cache_key(
        &hardware_json,
        &params.project,
        &params.git_ref,
        None,
        None,
        None,
    );

    let mut cache_hit = false;
    let mut cached_id = String::new();
    let mut cached_sha = String::new();

    {
        let conn = state.conn.lock().unwrap();
        if let Ok(Some(job_id)) = db::get_cache_entry(&conn, &cache_key) {
            if let Ok(Some(job_status)) = db::get_job_status(&conn, &job_id) {
                if job_status.status == "done" {
                    if let Ok(Some((file_path, _, sha256))) = db::get_artifact(&conn, &job_id) {
                        let path = std::path::Path::new(&file_path);
                        if path.exists() {
                            cache_hit = true;
                            cached_id = job_id;
                            cached_sha = sha256;
                        }
                    }
                }
            }
        }
    }

    if cache_hit {
        let download_url = format!("/build/{}/binary", cached_id);
        let response_body = serde_json::json!({
            "status": "cached",
            "download_url": download_url,
            "sha256": cached_sha
        });
        return (StatusCode::OK, Json(response_body)).into_response();
    }

    // 3. Create job entity (for cache miss)
    let job_id = Uuid::new_v4().to_string();
    let job = Job {
        id: job_id.clone(),
        token_id: token.id,
        project: params.project.clone(),
        git_ref: params.git_ref.clone(),
        hardware,
        binary: None,
        package: None,
        target: None,
        job_type: "standard".to_string(),
        pgo_source_job_id: None,
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
            &job.job_type,
            job.pgo_source_job_id.as_deref(),
        ) {
            return (StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to record job in database: {}", e)).into_response();
        }
    }

    // 5. Enqueue job
    match state.queue.enqueue(job) {
        Ok(id) => {
            let response_body = serde_json::json!({
                "status": "building",
                "job_id": id
            });
            (StatusCode::OK, Json(response_body)).into_response()
        }
        Err(QueueError::QueueFull) => {
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
