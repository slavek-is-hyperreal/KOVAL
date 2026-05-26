use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
    extract::Multipart,
};
use std::path::PathBuf;
use crate::routes::AppState;
use crate::auth;
use crate::db;
use crate::queue::{Job, QueueError};

pub async fn upload_profiles_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(instrument_job_id): Path<String>,
    mut multipart: Multipart,
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

    // 2. Fetch the instrumentation job details
    let job_details = {
        let conn = state.conn.lock().unwrap();
        match db::get_job_details(&conn, &instrument_job_id) {
            Ok(Some(details)) => details,
            Ok(None) => return (StatusCode::NOT_FOUND, "Job not found").into_response(),
            Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, format!("Database error: {}", e)).into_response(),
        }
    };

    // 3. Verify job ownership, type, and state
    if job_details.token_id != token.id {
        return (StatusCode::NOT_FOUND, "Job not found").into_response();
    }

    if job_details.job_type != "pgo_instrument" {
        return (StatusCode::BAD_REQUEST, "Job is not a PGO instrumentation job").into_response();
    }

    if job_details.status != "done" {
        return (StatusCode::BAD_REQUEST, "Instrumentation job is not in 'done' state").into_response();
    }

    // 4. Resolve profiles directory
    let profiles_dir = {
        let conn = state.conn.lock().unwrap();
        match db::get_pgo_profile(&conn, &instrument_job_id) {
            Ok(Some(p)) => PathBuf::from(p.profiles_dir),
            Ok(None) => {
                let dir = state.artifacts_dir.join("pgo_profiles").join(&instrument_job_id);
                if let Err(e) = std::fs::create_dir_all(&dir) {
                    return (StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to create profiles directory: {}", e)).into_response();
                }
                if let Err(e) = db::insert_pgo_profile(&conn, &instrument_job_id, &dir.to_string_lossy(), &now.to_rfc3339()) {
                    return (StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to insert profile record: {}", e)).into_response();
                }
                dir
            }
            Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, format!("Database error: {}", e)).into_response(),
        }
    };

    // 5. Read and validate multipart fields
    let mut files_saved = 0;
    while let Ok(Some(field)) = multipart.next_field().await {
        if files_saved >= 32 {
            return (StatusCode::BAD_REQUEST, "Maximum of 32 profile files is allowed").into_response();
        }
        let file_name = match field.file_name() {
            Some(name) => name.to_string(),
            None => continue,
        };

        if !file_name.ends_with(".profraw") {
            return (StatusCode::BAD_REQUEST, "Only files with .profraw extension are accepted").into_response();
        }

        let data = match field.bytes().await {
            Ok(b) => b,
            Err(e) => return e.into_response(),
        };

        let clean_name = match std::path::Path::new(&file_name).file_name() {
            Some(n) => n,
            None => continue,
        };

        let dest_path = profiles_dir.join(clean_name);
        if let Err(e) = std::fs::write(&dest_path, &data) {
            return (StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to save profile file: {}", e)).into_response();
        }
        files_saved += 1;
    }

    if files_saved == 0 {
        return (StatusCode::BAD_REQUEST, "No files uploaded").into_response();
    }

    // 6. Merge raw profiles using llvm-profdata
    let merged_path = profiles_dir.join("merged.profdata");
    let mut cmd = match crate::pgo::merge_command(&profiles_dir, &merged_path) {
        Ok(c) => c,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to build merge command: {}", e)).into_response(),
    };

    let output_result = tokio::task::spawn_blocking(move || cmd.output()).await;

    let output = match output_result {
        Ok(Ok(o)) => o,
        Ok(Err(e)) => return (StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to execute llvm-profdata: {}", e)).into_response(),
        Err(join_err) => return (StatusCode::INTERNAL_SERVER_ERROR, format!("Internal task join error: {}", join_err)).into_response(),
    };

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
        return (StatusCode::INTERNAL_SERVER_ERROR, format!("llvm-profdata merge failed: {}", stderr)).into_response();
    }

    // 7. Update merged path in DB
    {
        let conn = state.conn.lock().unwrap();
        if let Err(e) = db::update_pgo_merged_path(&conn, &instrument_job_id, &merged_path.to_string_lossy()) {
            return (StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to update merged path: {}", e)).into_response();
        }
    }

    // 8. Create and enqueue PGO Optimization Job
    let opt_job_id = uuid::Uuid::new_v4().to_string();
    let opt_job = Job {
        id: opt_job_id.clone(),
        token_id: token.id,
        project: job_details.project.clone(),
        git_ref: job_details.git_ref.clone(),
        hardware: job_details.hardware.clone(),
        binary: None,
        package: None,
        target: None,
        job_type: "pgo_optimize".to_string(),
        pgo_source_job_id: Some(instrument_job_id.clone()),
    };

    {
        let conn = state.conn.lock().unwrap();
        if let Err(e) = db::insert_job(
            &conn,
            &opt_job.id,
            opt_job.token_id,
            &opt_job.project,
            &opt_job.git_ref,
            &opt_job.hardware,
            &chrono::Utc::now().to_rfc3339(),
            &opt_job.job_type,
            opt_job.pgo_source_job_id.as_deref(),
        ) {
            return (StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to insert optimization job: {}", e)).into_response();
        }
    }

    match state.queue.enqueue(opt_job) {
        Ok(id) => {
            let response = schema::PgoUploadResponse {
                merged_profile_url: format!("/pgo/profiles/{}/merged.profdata", instrument_job_id),
                optimization_job_id: id,
            };
            (StatusCode::ACCEPTED, Json(response)).into_response()
        }
        Err(QueueError::QueueFull) => {
            {
                let conn = state.conn.lock().unwrap();
                let finished_now = chrono::Utc::now().to_rfc3339();
                db::update_job_status(
                    &conn,
                    &opt_job_id,
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

pub async fn get_merged_profile_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(instrument_job_id): Path<String>,
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
            Err(_) => return (StatusCode::UNAUTHORIZED, "Invalid or unauthorized token").into_response(),
        }
    };

    let profile = {
        let conn = state.conn.lock().unwrap();
        match db::get_pgo_profile(&conn, &instrument_job_id) {
            Ok(Some(p)) => p,
            _ => return (StatusCode::NOT_FOUND, "Profile not found").into_response(),
        }
    };

    let job_details = {
        let conn = state.conn.lock().unwrap();
        match db::get_job_details(&conn, &instrument_job_id) {
            Ok(Some(details)) => details,
            _ => return (StatusCode::NOT_FOUND, "Job not found").into_response(),
        }
    };

    if job_details.token_id != token.id {
        return (StatusCode::NOT_FOUND, "Profile not found").into_response();
    }

    let merged_path = match profile.merged_path {
        Some(path) => PathBuf::from(path),
        None => return (StatusCode::NOT_FOUND, "Merged profile data not generated yet").into_response(),
    };

    if !merged_path.exists() {
        return (StatusCode::NOT_FOUND, "Merged profile file does not exist on disk").into_response();
    }

    match std::fs::read(&merged_path) {
        Ok(bytes) => (
            StatusCode::OK,
            [(axum::http::header::CONTENT_TYPE, "application/octet-stream")],
            bytes,
        )
            .into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("Failed to read profile: {}", e)).into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_pgo_spawn_blocking_merge_case_30() {
        let mut cmd = std::process::Command::new("echo");
        cmd.arg("hello");

        let res = tokio::task::spawn_blocking(move || cmd.output()).await;
        assert!(res.is_ok());
        let output_res = res.unwrap();
        assert!(output_res.is_ok());
        let output = output_res.unwrap();
        assert!(output.status.success());
        assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "hello");
    }
}
